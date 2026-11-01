//! `kith ui`: hands over to the separate `kith-ui` program, so `kith` itself carries no HTTP.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::exec::CliError;

const PROGRAM: &str = "kith-ui";

/// `kith-ui` beside this executable, else the first on `PATH`.
fn find() -> Option<PathBuf> {
    let name = format!("{PROGRAM}{}", std::env::consts::EXE_SUFFIX);
    let beside = std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(&name))
        .filter(|path| path.is_file());
    beside.or_else(|| {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(&name))
            .find(|path| path.is_file())
    })
}

/// Runs `kith-ui` with `args`, passing on `--socket` when it was given to `kith`.
pub(crate) fn run(socket: Option<&Path>, args: &[String]) -> Result<(), CliError> {
    let program = find().ok_or_else(|| {
        CliError(format!(
            "{PROGRAM} is not installed; put the {PROGRAM} program from the kith release next to kith or on your PATH, or build it with: cargo install --path crates/management/kith-ui"
        ))
    })?;
    let mut command = Command::new(&program);
    if let Some(socket) = socket {
        command.arg("--socket").arg(socket);
    }
    command.args(args);
    exec(command, &program)
}

#[cfg(unix)]
fn exec(mut command: Command, program: &Path) -> Result<(), CliError> {
    use std::os::unix::process::CommandExt as _;
    let error = command.exec();
    Err(CliError(format!(
        "cannot run {}: {error}",
        program.display()
    )))
}

#[cfg(not(unix))]
fn exec(mut command: Command, program: &Path) -> Result<(), CliError> {
    let status = command
        .status()
        .map_err(|error| CliError(format!("cannot run {}: {error}", program.display())))?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError(format!("{PROGRAM} stopped with {status}")))
    }
}
