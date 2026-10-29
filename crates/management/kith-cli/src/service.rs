//! `kith up` and `kith down`: start or stop the background service, and keep it that
//! way across reboots. Asks for the password through sudo when not run as root.

use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::exec::CliError;

const LAUNCHD_LABEL: &str = "system/dev.kith.daemon";
const LAUNCHD_PLIST: &str = "/Library/LaunchDaemons/dev.kith.daemon.plist";
const SYSTEMD_UNIT: &str = "kith";
/// How long `kith up` waits for the daemon to answer.
const START_WAIT: Duration = Duration::from_secs(10);

/// Starts the service and enables it at boot, waiting until `socket` appears.
pub(crate) fn up(socket: &Path, out: &mut impl Write) -> Result<(), CliError> {
    if !is_root() {
        return rerun_as_root("up");
    }
    if cfg!(target_os = "macos") {
        require_installed(Path::new(LAUNCHD_PLIST))?;
        run("launchctl", &["enable", LAUNCHD_LABEL])?;
        if !succeeds("launchctl", &["print", LAUNCHD_LABEL]) {
            run("launchctl", &["bootstrap", "system", LAUNCHD_PLIST])?;
        }
    } else if cfg!(target_os = "linux") {
        run("systemctl", &["enable", "--now", SYSTEMD_UNIT])?;
    } else {
        return Err(unsupported());
    }
    wait_for(socket)?;
    writeln!(out, "kith is on")?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the CLI is a driver: it may block and read the clock"
)]
fn wait_for(socket: &Path) -> Result<(), CliError> {
    let started = Instant::now();
    while !socket.exists() {
        if started.elapsed() > START_WAIT {
            return Err(CliError(
                "the service started but the daemon is not answering; run `kith doctor`".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Stops the service and keeps it off across reboots, until `kith up`.
pub(crate) fn down(out: &mut impl Write) -> Result<(), CliError> {
    if !is_root() {
        return rerun_as_root("down");
    }
    if cfg!(target_os = "macos") {
        run("launchctl", &["disable", LAUNCHD_LABEL])?;
        if succeeds("launchctl", &["print", LAUNCHD_LABEL]) {
            run("launchctl", &["bootout", LAUNCHD_LABEL])?;
        }
    } else if cfg!(target_os = "linux") {
        run("systemctl", &["disable", "--now", SYSTEMD_UNIT])?;
    } else {
        return Err(unsupported());
    }
    writeln!(
        out,
        "kith is off, and stays off after a restart until `kith up`"
    )?;
    Ok(())
}

fn is_root() -> bool {
    kith_sys::effective_uid() == Some(0)
}

fn rerun_as_root(command: &str) -> Result<(), CliError> {
    let me = std::env::current_exe()
        .map_err(|e| CliError(format!("cannot find the kith binary: {e}")))?;
    let status = Command::new("sudo")
        .arg(me)
        .arg(command)
        .status()
        .map_err(|e| CliError(format!("cannot run sudo: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError(format!("`sudo kith {command}` did not finish")))
    }
}

fn require_installed(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        Ok(())
    } else {
        Err(CliError(format!(
            "the kith service is not installed ({} is missing)",
            path.display()
        )))
    }
}

fn unsupported() -> CliError {
    CliError("`kith up` and `kith down` support macOS and Linux; start `kith daemon` with your service manager".into())
}

fn succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run(program: &str, args: &[&str]) -> Result<(), CliError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| CliError(format!("cannot run {program}: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CliError(format!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}
