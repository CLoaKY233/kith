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
const SYSTEMD_UNIT_PATH: &str = "/etc/systemd/system/kith.service";
/// The service definitions shipped in `packaging/`, written by `kith up` when missing.
const LAUNCHD_TEMPLATE: &str = include_str!("../../../../packaging/launchd/dev.kith.daemon.plist");
const SYSTEMD_TEMPLATE: &str = include_str!("../../../../packaging/systemd/kith.service");
/// How long `kith up` waits for the daemon to answer.
const START_WAIT: Duration = Duration::from_secs(10);

/// Starts the service and enables it at boot, waiting until `socket` appears.
pub(crate) fn up(socket: &Path, out: &mut impl Write) -> Result<(), CliError> {
    if !is_root() {
        return rerun_as_root("up");
    }
    if cfg!(target_os = "macos") {
        install_service(
            Path::new(LAUNCHD_PLIST),
            LAUNCHD_TEMPLATE,
            "/usr/local/bin/kith",
        )?;
        run("launchctl", &["enable", LAUNCHD_LABEL])?;
        if !succeeds("launchctl", &["print", LAUNCHD_LABEL]) {
            run("launchctl", &["bootstrap", "system", LAUNCHD_PLIST])?;
        }
    } else if cfg!(target_os = "linux") {
        if install_service(
            Path::new(SYSTEMD_UNIT_PATH),
            SYSTEMD_TEMPLATE,
            "/usr/bin/kith",
        )? {
            run("systemctl", &["daemon-reload"])?;
        }
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

/// Writes the service definition at `path` from `template`, pointing it at this binary
/// instead of `placeholder`, unless one is there already. Returns whether it wrote one.
pub(crate) fn install_service(
    path: &Path,
    template: &str,
    placeholder: &str,
) -> Result<bool, CliError> {
    if path.exists() {
        return Ok(false);
    }
    let me = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| CliError(format!("cannot find the kith binary: {e}")))?;
    let me = me
        .to_str()
        .filter(|me| !me.contains(['<', '>', '&', '"', '\'', ' ', '\n']))
        .ok_or_else(|| {
            CliError(format!(
                "move kith to a path without spaces or symbols (it is at {})",
                me.display()
            ))
        })?;
    std::fs::write(path, template.replace(placeholder, me))
        .map_err(|e| CliError(format!("cannot write {}: {e}", path.display())))?;
    Ok(true)
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
