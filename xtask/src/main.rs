//! Repository automation, run as `cargo xtask <command>`.
//!
//! - `ci`: every check CI runs, in the same order.
//! - `no-std`: build every sans-IO core crate for a bare-metal target.
//! - `core-crates`: print the sans-IO core crates (those marked `#![no_std]`).
//! - `size`: build `kith` with the `embedded` profile and enforce the size budget.

#![forbid(unsafe_code)]

use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Default binary size budget: 5 MB.
const DEFAULT_MAX_BYTES: u64 = 5_000_000;

/// Bare-metal target used to prove the core crates never touch `std`.
const NO_STD_TARGET: &str = "thumbv7em-none-eabihf";

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci              run every CI check locally
  no-std          build every core crate for a bare-metal target
  core-crates     print the sans-IO core crates, one per line
  size [--target T] [--max-bytes N] [--zigbuild]
                  build `kith` (embedded profile) and enforce the size budget";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("no-std") => no_std(),
        Some("core-crates") => print_core_crates(),
        Some("size") => size(rest),
        Some("help" | "--help" | "-h") | None => {
            say(USAGE);
            Ok(())
        }
        Some(other) => Err(Error::Usage(format!("unknown command `{other}`"))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            say(&format!("xtask: {error}"));
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug)]
enum Error {
    Usage(String),
    Io(io::Error),
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "{message}\n\n{USAGE}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Failed(step) => write!(f, "step failed: {step}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Writes a line to stderr. Failing to report is not itself reportable, so it is ignored.
fn say(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ignored = writeln!(stderr, "{message}");
}

fn workspace_root() -> PathBuf {
    // xtask lives one level below the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Runs `program args...` from the workspace root, failing if it exits non-zero.
fn run<S: AsRef<str>>(program: &str, args: &[S]) -> Result<(), Error> {
    run_with_env(program, args, &[])
}

/// Like [`run`], with extra environment variables.
fn run_with_env<S: AsRef<str>>(
    program: &str,
    args: &[S],
    vars: &[(&str, &str)],
) -> Result<(), Error> {
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let line = format!("{program} {}", args.join(" "));
    say(&format!("\n==> {line}"));
    let status = Command::new(program)
        .args(&args)
        .envs(vars.iter().copied())
        .current_dir(workspace_root())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Failed(line))
    }
}

fn ci() -> Result<(), Error> {
    run("cargo", &["fmt", "--all", "--check"])?;
    run("taplo", &["fmt", "--check"])?;
    run::<&str>("typos", &[])?;
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(
        "cargo",
        &[
            "nextest",
            "run",
            "--workspace",
            "--all-features",
            "--locked",
            "--no-tests=pass",
        ],
    )?;
    run(
        "cargo",
        &["test", "--workspace", "--doc", "--all-features", "--locked"],
    )?;
    run_with_env(
        "cargo",
        &[
            "doc",
            "--workspace",
            "--no-deps",
            "--all-features",
            "--locked",
        ],
        &[("RUSTDOCFLAGS", "-D warnings")],
    )?;
    no_std()?;
    run("cargo", &["deny", "--all-features", "check"])?;
    run("cargo-machete", &["--with-metadata", "."])?;
    say("\nall checks passed");
    Ok(())
}

/// Finds the core crates: every `crates/*/*` whose `src/lib.rs` declares `#![no_std]`.
fn core_crates() -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    let mut dirs = Vec::new();
    for group in fs::read_dir(workspace_root().join("crates"))? {
        for entry in fs::read_dir(group?.path())? {
            dirs.push(entry?.path());
        }
    }
    for dir in dirs {
        let lib = dir.join("src").join("lib.rs");
        let Ok(source) = fs::read_to_string(&lib) else {
            continue;
        };
        if source.lines().any(|line| line.trim() == "#![no_std]")
            && let Some(name) = dir.file_name().and_then(|name| name.to_str())
        {
            names.push(name.to_owned());
        }
    }
    names.sort_unstable();
    Ok(names)
}

fn print_core_crates() -> Result<(), Error> {
    let mut stdout = io::stdout().lock();
    for name in core_crates()? {
        writeln!(stdout, "{name}")?;
    }
    Ok(())
}

fn no_std() -> Result<(), Error> {
    let mut args = vec![
        "build".to_owned(),
        "--locked".to_owned(),
        "--target".to_owned(),
        NO_STD_TARGET.to_owned(),
    ];
    for name in core_crates()? {
        args.push("--package".to_owned());
        args.push(name);
    }
    run("cargo", &args)
}

fn size(args: &[String]) -> Result<(), Error> {
    let mut target: Option<&str> = None;
    let mut max_bytes = DEFAULT_MAX_BYTES;
    let mut zigbuild = false;
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--zigbuild" => zigbuild = true,
            "--target" | "--max-bytes" => {
                let value = rest
                    .next()
                    .ok_or_else(|| Error::Usage(format!("`{flag}` needs a value")))?;
                if flag == "--target" {
                    target = Some(value);
                } else {
                    max_bytes = value
                        .parse()
                        .map_err(|_| Error::Usage(format!("invalid byte count `{value}`")))?;
                }
            }
            other => return Err(Error::Usage(format!("unknown flag `{other}`"))),
        }
    }

    let mut build = vec![
        if zigbuild { "zigbuild" } else { "build" },
        "--package",
        "kith-cli",
        "--profile",
        "embedded",
        "--locked",
    ];
    if let Some(triple) = target {
        build.extend(["--target", triple]);
    }
    run("cargo", &build)?;

    let mut binary = workspace_root().join("target");
    if let Some(triple) = target {
        binary.push(triple);
    }
    binary.push("embedded");
    let windows = target.map_or(cfg!(windows), |triple| triple.contains("windows"));
    binary.push(if windows { "kith.exe" } else { "kith" });

    let bytes = fs::metadata(&binary)?.len();
    say(&format!(
        "{}: {bytes} bytes (budget {max_bytes})",
        binary.display()
    ));
    if bytes > max_bytes {
        return Err(Error::Failed(format!(
            "binary is {bytes} bytes, over the {max_bytes}-byte budget"
        )));
    }
    Ok(())
}
