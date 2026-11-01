//! Compiles the built page, `web/dist`, into the binary, each file also brotli-compressed
//! at quality 11. Without a complete built page it embeds `fallback.html` and warns; the
//! build never fails for want of Node.

use std::error::Error;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Brotli's best and slowest quality: compression happens once, at build time.
const QUALITY: u32 = 11;
/// Brotli's largest window.
const WINDOW: u32 = 24;
/// The places in `index.html` that `kith-ui-core` fills on every request: the boot snapshot
/// and the signed-out page.
const MARKERS: [&str; 2] = ["<!--kith:boot-->", "<!--kith:signed-out-->"];

/// Every file under `dir`, skipping dotfiles such as `.DS_Store` so they never ship, and
/// symlinks, which could pull in files from outside `web/dist` or loop forever.
fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink()
            || path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if kind.is_dir() {
            collect(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

/// Root-relative paths the page loads through `src="/..."` or `href="/..."`, without any
/// query or fragment, which name the same file.
fn references(html: &str) -> Vec<&str> {
    ["src=\"/", "href=\"/"]
        .iter()
        .flat_map(|attribute| html.split(attribute).skip(1))
        .filter_map(|rest| rest.split('"').next())
        .filter_map(|path| path.split(['?', '#']).next())
        .filter(|path| !path.is_empty() && !path.starts_with('/'))
        .collect()
}

/// Why `web/dist` cannot be embedded, or `None` when it is complete. A page whose scripts
/// or styles are missing would load blank, so the fallback is better.
fn incomplete(dist: &Path) -> std::io::Result<Option<String>> {
    let index = dist.join("index.html");
    if !index.is_file() {
        return Ok(Some("web/dist is missing".to_owned()));
    }
    let html = std::fs::read_to_string(&index)?;
    Ok(references(&html)
        .into_iter()
        .find(|path| !dist.join(path).is_file())
        .map(|missing| format!("web/dist/index.html loads /{missing}, which is not in web/dist")))
}

/// Writes `bytes` only when they differ, so an unchanged page does not rebuild the binary.
fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    std::fs::write(path, bytes)
}

fn brotli(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut out, 64 * 1024, QUALITY, WINDOW);
        writer.write_all(bytes)?;
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let dist = manifest.join("web").join("dist");
    let fallback = manifest.join("fallback.html");
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "cargo::rerun-if-changed=web/dist")?;
    writeln!(stdout, "cargo::rerun-if-changed=fallback.html")?;

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if let Some(reason) = incomplete(&dist)? {
        writeln!(
            stdout,
            "cargo::warning={reason}, so kith-ui embeds fallback.html. Run `npm ci && npm run build` in crates/management/kith-ui/web."
        )?;
        files.push(("index.html".to_owned(), fallback));
    } else {
        let html = std::fs::read_to_string(dist.join("index.html"))?;
        for marker in MARKERS.iter().filter(|marker| !html.contains(*marker)) {
            writeln!(
                stdout,
                "cargo::warning=web/dist/index.html lacks {marker}, which kith-ui fills on every request."
            )?;
        }
        let mut paths = Vec::new();
        collect(&dist, &mut paths)?;
        paths.sort();
        for path in paths {
            let relative = path
                .strip_prefix(&dist)?
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            files.push((relative, path));
        }
    }

    let compressed = out.join("brotli");
    std::fs::create_dir_all(&compressed)?;
    let mut table = String::from("&[\n");
    for (index, (relative, path)) in files.iter().enumerate() {
        let bytes = std::fs::read(path)?;
        let packed = brotli(&bytes)?;
        let brotli = if packed.len() < bytes.len() && relative != "index.html" {
            let target = compressed.join(format!("{index}.br"));
            write_if_changed(&target, &packed)?;
            format!("Some(include_bytes!({:?}))", target.display().to_string())
        } else {
            "None".to_owned()
        };
        writeln!(
            table,
            "    Asset {{ path: {relative:?}, body: include_bytes!({:?}), brotli: {brotli} }},",
            path.display().to_string()
        )?;
    }
    table.push(']');
    write_if_changed(&out.join("assets.rs"), table.as_bytes())?;
    Ok(())
}
