//! Runs Tauri's build step, then compiles the kith page into the binary the same way
//! `kith-ui`'s build script does: `web/dist`, each file also brotli-compressed, or
//! `fallback.html` with a warning when the page was not built. The build never needs Node.

use std::error::Error;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Brotli's best and slowest quality: compression happens once, at build time.
const QUALITY: u32 = 11;
/// Brotli's largest window.
const WINDOW: u32 = 24;

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
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

fn embed_page() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let ui = manifest.join("../../crates/management/kith-ui");
    let dist = ui.join("web").join("dist");
    let fallback = ui.join("fallback.html");
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "cargo::rerun-if-changed={}", dist.display())?;
    writeln!(stdout, "cargo::rerun-if-changed={}", fallback.display())?;

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if dist.join("index.html").is_file() {
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
    } else {
        writeln!(
            stdout,
            "cargo::warning=kith-ui/web/dist is missing, so kith-app embeds fallback.html. Run `npm ci && npm run build` in crates/management/kith-ui/web."
        )?;
        files.push(("index.html".to_owned(), fallback));
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

fn main() -> Result<(), Box<dyn Error>> {
    tauri_build::build();
    embed_page()
}
