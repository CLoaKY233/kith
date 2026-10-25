//! The `kith` binary: daemon and command line in one executable.
//!
//! Plane: management. Binary. See `docs/architecture.md`.

#![forbid(unsafe_code)]
use std::io::Write as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match writeln!(stdout, "kith {}", env!("CARGO_PKG_VERSION")) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
