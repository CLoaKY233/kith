//! Filesystem store with crash-safe atomic writes.

use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    Error, Key, Name, Namespace, Result, Secrecy, Store,
    format::{self, HEADER_LEN, MAX_VALUE_LEN},
};

/// Prefix of value file names; `@` never appears in a segment.
const VALUE_PREFIX: &str = "@";

/// Prefix of temporary files; a leading dot never appears in a segment.
const TEMP_PREFIX: &str = ".tmp.";

/// Distinguishes temp files written concurrently by one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A [`Store`] that keeps one file per value under a root directory.
///
/// See the crate docs for the layout and the file format.
#[derive(Debug)]
pub struct FsStore {
    root: PathBuf,
}

impl FsStore {
    /// Opens (creating if needed, mode `0700` on Unix) the store at `root`
    /// and deletes temporary files left behind by earlier crashes.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let mut builder = DirBuilder::new();
        builder.recursive(true);
        set_dir_mode(&mut builder, Secrecy::Secret);
        builder
            .create(&root)
            .map_err(Error::io("create directory", &root))?;
        let store = Self { root };
        store.remove_stale_temp_files()?;
        Ok(store)
    }

    /// The root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn namespace_dir(&self, namespace: &Namespace) -> PathBuf {
        let mut dir = self.root.clone();
        dir.extend(namespace.segments());
        dir
    }

    fn value_path(&self, key: &Key) -> PathBuf {
        self.namespace_dir(&key.namespace)
            .join(value_file_name(&key.name))
    }

    /// Walks the tree and removes every `.tmp.*` file.
    fn remove_stale_temp_files(&self) -> Result<()> {
        let mut pending = vec![self.root.clone()];
        while let Some(dir) = pending.pop() {
            let mut removed_any = false;
            for entry in fs::read_dir(&dir).map_err(Error::io("read directory", &dir))? {
                let entry = entry.map_err(Error::io("read directory", &dir))?;
                let path = entry.path();
                let kind = entry.file_type().map_err(Error::io("stat", &path))?;
                if kind.is_dir() {
                    pending.push(path);
                } else if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(TEMP_PREFIX))
                {
                    fs::remove_file(&path).map_err(Error::io("remove stale temp file", &path))?;
                    removed_any = true;
                }
            }
            if removed_any {
                sync_dir(&dir)?;
            }
        }
        Ok(())
    }

    /// Creates the namespace directories one segment at a time, flushing
    /// each parent whose entries changed so the new directory is durable.
    fn ensure_namespace_dir(&self, namespace: &Namespace, secrecy: Secrecy) -> Result<PathBuf> {
        let mut builder = DirBuilder::new();
        set_dir_mode(&mut builder, secrecy);
        let mut dir = self.root.clone();
        for segment in namespace.segments() {
            let parent = dir.clone();
            dir.push(segment);
            match builder.create(&dir) {
                Ok(()) => sync_dir(&parent)?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(Error::io("create directory", &dir)(error)),
            }
        }
        Ok(dir)
    }
}

fn value_file_name(name: &Name) -> String {
    format!("{VALUE_PREFIX}{name}")
}

impl Store for FsStore {
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>> {
        let path = self.value_path(key);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io("open", &path)(error)),
        };
        // Read at most one byte past the largest valid file, so an oversized
        // (corrupt) file is detected without allocating its full size.
        let limit = HEADER_LEN.saturating_add(MAX_VALUE_LEN).saturating_add(1);
        let mut bytes = Vec::new();
        file.take(u64::try_from(limit).unwrap_or(u64::MAX))
            .read_to_end(&mut bytes)
            .map_err(Error::io("read", &path))?;
        match format::decode(&bytes) {
            Ok(payload) => Ok(Some(payload.to_vec())),
            Err(reason) => Err(Error::Corrupt { path, reason }),
        }
    }

    fn put_with(&self, key: &Key, value: &[u8], secrecy: Secrecy) -> Result<()> {
        let length = format::checked_length(value)?;
        let encoded = format::encode(value, length);
        let dir = self.ensure_namespace_dir(&key.namespace, secrecy)?;
        let file_name = value_file_name(&key.name);
        let target = dir.join(&file_name);
        let temp = dir.join(format!(
            "{TEMP_PREFIX}{file_name}.{}.{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(error) = write_synced(&temp, &encoded, secrecy) {
            // Best effort: a leftover is also removed on the next open.
            drop(fs::remove_file(&temp));
            return Err(error);
        }
        if let Err(source) = fs::rename(&temp, &target) {
            drop(fs::remove_file(&temp));
            return Err(Error::Io {
                op: "rename",
                path: target,
                source,
            });
        }
        sync_dir(&dir)
    }

    fn delete(&self, key: &Key) -> Result<()> {
        let path = self.value_path(key);
        match fs::remove_file(&path) {
            Ok(()) => sync_dir(&self.namespace_dir(&key.namespace)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::io("remove", &path)(error)),
        }
    }

    fn list(&self, namespace: &Namespace) -> Result<Vec<Key>> {
        let dir = self.namespace_dir(namespace);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::io("read directory", &dir)(error)),
        };
        let mut keys = Vec::new();
        for entry in entries {
            let entry = entry.map_err(Error::io("read directory", &dir))?;
            let file_name = entry.file_name();
            // Anything that is not `@<valid name>` (temp files, foreign files,
            // sub-namespace directories) is not a key in this namespace.
            let Some(name) = file_name
                .to_str()
                .and_then(|name| name.strip_prefix(VALUE_PREFIX))
                .and_then(|name| Name::new(name).ok())
            else {
                continue;
            };
            keys.push(Key::new(namespace.clone(), name));
        }
        keys.sort();
        Ok(keys)
    }
}

/// Creates `path` (which must not exist), writes `bytes` and flushes them.
fn write_synced(path: &Path, bytes: &[u8], secrecy: Secrecy) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    set_file_mode(&mut options, secrecy);
    let mut file = options.open(path).map_err(Error::io("create", path))?;
    file.write_all(bytes).map_err(Error::io("write", path))?;
    file.sync_all().map_err(Error::io("sync", path))
}

#[cfg(unix)]
fn set_file_mode(options: &mut OpenOptions, secrecy: Secrecy) {
    use std::os::unix::fs::OpenOptionsExt as _;
    options.mode(match secrecy {
        Secrecy::Public => 0o644,
        Secrecy::Secret => 0o600,
    });
}

#[cfg(not(unix))]
fn set_file_mode(_options: &mut OpenOptions, _secrecy: Secrecy) {}

#[cfg(unix)]
fn set_dir_mode(builder: &mut DirBuilder, secrecy: Secrecy) {
    use std::os::unix::fs::DirBuilderExt as _;
    builder.mode(match secrecy {
        Secrecy::Public => 0o755,
        Secrecy::Secret => 0o700,
    });
}

#[cfg(not(unix))]
fn set_dir_mode(_builder: &mut DirBuilder, _secrecy: Secrecy) {}

/// Flushes a directory's entries, making a rename or unlink in it durable.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)
        .and_then(|handle| handle.sync_all())
        .map_err(Error::io("sync directory", dir))
}

/// Windows cannot open a directory for flushing through safe std APIs; NTFS
/// journals the rename itself.
#[cfg(not(unix))]
#[expect(clippy::unnecessary_wraps, reason = "matches the Unix signature")]
fn sync_dir(_dir: &Path) -> Result<()> {
    Ok(())
}
