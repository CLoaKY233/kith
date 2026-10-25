//! Crate-level tests shared by both store implementations.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use proptest::prelude::*;

use crate::{
    Error, FsStore, Key, KeyError, MAX_VALUE_LEN, MemoryStore, Name, Namespace, Store,
    format::{decode, encode},
};

/// A unique directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "kith-store-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        drop(fs::remove_dir_all(&path));
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

fn key(namespace: &str, name: &str) -> Key {
    Key::parse(namespace, name).unwrap()
}

/// The behavioural contract every `Store` must meet.
fn contract(store: &dyn Store) {
    let alpha = key("settings", "alpha");
    let beta = key("settings", "beta");
    let nested = key("settings/sub", "alpha");

    assert_eq!(store.get(&alpha).unwrap(), None);
    store.put(&beta, b"2").unwrap();
    store.put(&alpha, b"1").unwrap();
    store.put_secret(&nested, b"secret").unwrap();
    assert_eq!(store.get(&alpha).unwrap().as_deref(), Some(&b"1"[..]));
    assert_eq!(store.get(&nested).unwrap().as_deref(), Some(&b"secret"[..]));

    store.put(&alpha, b"").unwrap();
    assert_eq!(store.get(&alpha).unwrap().as_deref(), Some(&b""[..]));

    let settings = Namespace::new("settings").unwrap();
    assert_eq!(
        store.list(&settings).unwrap(),
        vec![alpha.clone(), beta.clone()]
    );
    assert_eq!(
        store.list(&settings.join("sub").unwrap()).unwrap(),
        vec![nested]
    );
    assert_eq!(
        store.list(&Namespace::new("none").unwrap()).unwrap(),
        Vec::<Key>::new()
    );

    store.delete(&alpha).unwrap();
    store.delete(&alpha).unwrap();
    assert_eq!(store.get(&alpha).unwrap(), None);
    assert_eq!(store.list(&settings).unwrap(), vec![beta]);

    let too_big = vec![0; MAX_VALUE_LEN + 1];
    assert!(matches!(
        store.put(&alpha, &too_big),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn memory_store_meets_store_contract() {
    contract(&MemoryStore::new());
}

#[test]
fn fs_store_meets_store_contract() {
    let dir = TempDir::new();
    contract(&FsStore::open(dir.path()).unwrap());
}

#[test]
fn reopen_after_crash_mid_write_keeps_old_value_and_removes_temp_file() {
    let dir = TempDir::new();
    let target = key("ledger/ab12", "ops");
    let store = FsStore::open(dir.path()).unwrap();
    store.put(&target, b"old").unwrap();

    // Simulate a crash mid-overwrite: a half-written temp file next to the
    // target, never renamed.
    let namespace_dir = dir.path().join("ledger").join("ab12");
    let stale = namespace_dir.join(".tmp.@ops.999.0");
    fs::write(&stale, &encode(b"new", 3)[..7]).unwrap();
    drop(store);

    let store = FsStore::open(dir.path()).unwrap();
    assert!(!stale.exists());
    assert_eq!(store.get(&target).unwrap().as_deref(), Some(&b"old"[..]));
    assert_eq!(store.list(&target.namespace).unwrap(), vec![target.clone()]);
    store.put(&target, b"new").unwrap();
    assert_eq!(store.get(&target).unwrap().as_deref(), Some(&b"new"[..]));
    let leftovers: Vec<_> = fs::read_dir(&namespace_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec!["@ops"]);
}

#[test]
fn any_flipped_byte_is_reported_as_corrupt() {
    let dir = TempDir::new();
    let target = key("settings", "dns");
    let store = FsStore::open(dir.path()).unwrap();
    store.put(&target, b"hello world").unwrap();
    let path = dir.path().join("settings").join("@dns");
    let original = fs::read(&path).unwrap();
    for index in 0..original.len() {
        let mut bytes = original.clone();
        bytes[index] ^= 0x01;
        fs::write(&path, &bytes).unwrap();
        assert!(
            matches!(store.get(&target), Err(Error::Corrupt { .. })),
            "flip at {index} not detected"
        );
    }
}

#[test]
fn truncated_or_extended_file_is_reported_as_corrupt() {
    let dir = TempDir::new();
    let target = key("settings", "dns");
    let store = FsStore::open(dir.path()).unwrap();
    store.put(&target, b"hello world").unwrap();
    let path = dir.path().join("settings").join("@dns");
    let original = fs::read(&path).unwrap();
    for length in 0..original.len() {
        fs::write(&path, &original[..length]).unwrap();
        assert!(matches!(store.get(&target), Err(Error::Corrupt { .. })));
    }
    let mut extended = original;
    extended.push(0);
    fs::write(&path, &extended).unwrap();
    assert!(matches!(store.get(&target), Err(Error::Corrupt { .. })));
}

#[test]
fn on_disk_format_matches_golden_bytes() {
    let encoded = encode(b"hi", 2);
    let expected: [u8; 18] = [
        b'K', b'I', b'T', b'H', // magic
        1, 0, 0, 0, // version, reserved
        2, 0, 0, 0, // length
        0x27, 0x45, 0x94, 0x8D, // crc-32, cross-checked with zlib
        b'h', b'i',
    ];
    assert_eq!(encoded, expected);
    assert_eq!(decode(&expected).unwrap(), b"hi");
}

#[cfg(unix)]
#[test]
fn secret_values_are_owner_only_from_creation() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = TempDir::new();
    let store = FsStore::open(dir.path()).unwrap();
    store
        .put_secret(&key("device/keys", "private"), b"k")
        .unwrap();
    store.put(&key("settings", "public"), b"p").unwrap();
    let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(dir.path().to_path_buf()), 0o700);
    assert_eq!(mode(dir.path().join("device")), 0o700);
    assert_eq!(mode(dir.path().join("device/keys")), 0o700);
    assert_eq!(mode(dir.path().join("device/keys/@private")), 0o600);
    // Public files are at most 0644 (the umask may narrow them further).
    assert_eq!(mode(dir.path().join("settings/@public")) & !0o644, 0);
}

#[test]
fn keys_cannot_express_path_traversal() {
    let too_long = "a".repeat(129);
    for bad in [
        "",
        ".",
        "..",
        ".hidden",
        "a/b",
        "a\\b",
        "a\0b",
        "Upper",
        "caf\u{e9}",
        "a b",
        "@x",
        &too_long,
    ] {
        assert!(Name::new(bad).is_err(), "name {bad:?} accepted");
    }
    for bad in [
        "", "/", "a/", "/a", "a//b", "a/../b", "a/./b", "a/.x", "..", "A/b", "a\\b",
    ] {
        assert!(Namespace::new(bad).is_err(), "namespace {bad:?} accepted");
    }
    assert_eq!(Name::new(""), Err(KeyError::Empty));
    assert_eq!(Name::new(".."), Err(KeyError::LeadingDot));
    assert_eq!(Name::new(&too_long), Err(KeyError::TooLong));
    assert_eq!(Name::new("a/b"), Err(KeyError::InvalidByte(b'/')));
    assert!(Namespace::new(&["a"; 200].join("/")).is_err());

    assert!(Name::new(&"a".repeat(128)).is_ok());
    assert!(Name::new("ops.v1_x-2").is_ok());
    assert!(Name::new("a..b").is_ok());
    assert!(Namespace::new("ledger/0123abcd").is_ok());
}

fn name_strategy() -> impl Strategy<Value = String> {
    "[a-z0-9_-][a-z0-9._-]{0,15}"
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn arbitrary_key_strings_never_panic_and_accepted_ones_are_safe(input in any::<String>()) {
        if let Ok(name) = Name::new(&input) {
            prop_assert_eq!(name.as_str(), input.as_str());
            prop_assert!(!input.contains(['/', '\\', '\0']) && !input.starts_with('.'));
        }
        if let Ok(namespace) = Namespace::new(&input) {
            prop_assert!(namespace.segments().all(|segment| segment != ".." && segment != "."));
        }
    }

    #[test]
    fn decoding_arbitrary_bytes_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        let _ = decode(&bytes);
    }

    #[test]
    fn both_stores_match_btreemap_model(
        operations in proptest::collection::vec(
            (prop_oneof![Just("a"), Just("a/b"), Just("c")], name_strategy(),
             proptest::option::of(proptest::collection::vec(any::<u8>(), 0..64))),
            1..24,
        )
    ) {
        let dir = TempDir::new();
        let file_store = FsStore::open(dir.path()).unwrap();
        let memory_store = MemoryStore::new();
        let stores: [&dyn Store; 2] = [&file_store, &memory_store];
        let mut model = BTreeMap::new();
        for (namespace, name, value) in operations {
            let key = key(namespace, &name);
            for store in stores {
                match &value {
                    Some(value) => store.put(&key, value).unwrap(),
                    None => store.delete(&key).unwrap(),
                }
            }
            match value {
                Some(value) => { model.insert(key, value); }
                None => { model.remove(&key); }
            }
        }
        for namespace in ["a", "a/b", "c"] {
            let namespace = Namespace::new(namespace).unwrap();
            let expected: Vec<Key> =
                model.keys().filter(|key| key.namespace == namespace).cloned().collect();
            for store in stores {
                prop_assert_eq!(&store.list(&namespace).unwrap(), &expected);
            }
        }
        for (key, value) in &model {
            for store in stores {
                let stored = store.get(key).unwrap();
                prop_assert_eq!(stored.as_ref(), Some(value));
            }
        }
    }
}
