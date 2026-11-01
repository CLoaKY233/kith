//! Compiles every file under `web/` into the binary, so `kith-ui` is one file to install.

use std::error::Error;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

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

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("web");
    let mut files = Vec::new();
    collect(&root, &mut files)?;
    files.sort();
    let mut table = String::from("&[\n");
    for file in &files {
        let relative = file
            .strip_prefix(&root)?
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        writeln!(
            table,
            "    ({relative:?}, include_bytes!({:?})),",
            file.display().to_string()
        )?;
    }
    table.push(']');
    let out = PathBuf::from(std::env::var("OUT_DIR")?).join("web.rs");
    std::fs::write(out, table)?;
    writeln!(std::io::stdout().lock(), "cargo::rerun-if-changed=web")?;
    Ok(())
}
