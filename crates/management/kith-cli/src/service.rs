//! `kith up` and `kith down`: start or stop the background service, and keep it that
//! way across reboots. Asks for the password through sudo when not run as root.

use std::ffi::OsString;
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
        return rerun_as_root(&root_args("up", Some(socket)));
    }
    let stale = is_stale(running_version(socket).as_deref());
    if cfg!(target_os = "macos") {
        let rewritten = install_service(
            Path::new(LAUNCHD_PLIST),
            LAUNCHD_TEMPLATE,
            "/usr/local/bin/kith",
        )?;
        run("launchctl", &["enable", LAUNCHD_LABEL])?;
        let loaded = succeeds("launchctl", &["print", LAUNCHD_LABEL]);
        if loaded && rewritten {
            run("launchctl", &["bootout", LAUNCHD_LABEL])?;
        }
        if !loaded || rewritten {
            run("launchctl", &["bootstrap", "system", LAUNCHD_PLIST])?;
        } else if stale {
            run("launchctl", &["kickstart", "-k", LAUNCHD_LABEL])?;
        }
    } else if cfg!(target_os = "linux") {
        let rewritten = install_service(
            Path::new(SYSTEMD_UNIT_PATH),
            SYSTEMD_TEMPLATE,
            "/usr/bin/kith",
        )?;
        if rewritten {
            run("systemctl", &["daemon-reload"])?;
        }
        run("systemctl", &["enable", "--now", SYSTEMD_UNIT])?;
        if stale || rewritten {
            run("systemctl", &["restart", SYSTEMD_UNIT])?;
        }
    } else {
        return Err(unsupported());
    }
    wait_for(socket)?;
    if is_stale(running_version(socket).as_deref()) {
        return Err(CliError(
            "the daemon still runs another version of kith; run `kith down` and `kith up`".into(),
        ));
    }
    writeln!(out, "kith is on")?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the CLI is a driver: it may block and read the clock"
)]
fn wait_for(socket: &Path) -> Result<(), CliError> {
    let started = Instant::now();
    while !answers(socket) {
        if started.elapsed() > START_WAIT {
            return Err(CliError(
                "the service started but the daemon is not answering; run `kith doctor`".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Whether the daemon answering with `running` must restart onto this binary: it reports
/// a different version, or could not complete the handshake at all. `None` means it is
/// not running.
pub(crate) fn is_stale(running: Option<&str>) -> bool {
    running.is_some_and(|version| version != env!("CARGO_PKG_VERSION"))
}

/// The version of the daemon on `socket`: `None` when nothing answers, an empty string
/// when it answers but the handshake fails (another protocol version).
fn running_version(socket: &Path) -> Option<String> {
    if !answers(socket) {
        return None;
    }
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return None;
    };
    let version = runtime.block_on(async {
        match tokio::time::timeout(Duration::from_secs(3), kith_ipc::IpcClient::connect(socket))
            .await
        {
            Ok(Ok(client)) => client.daemon_version().to_owned(),
            Ok(Err(_)) | Err(_) => String::new(),
        }
    });
    Some(version)
}

/// Whether a daemon accepts connections on `socket`. A socket file alone proves nothing:
/// one left behind by a daemon that crashed is still there.
#[cfg(unix)]
pub(crate) fn answers(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

#[cfg(not(unix))]
pub(crate) fn answers(socket: &Path) -> bool {
    socket.exists()
}

/// Stops the service and keeps it off across reboots, until `kith up`.
pub(crate) fn down(out: &mut impl Write) -> Result<(), CliError> {
    if !is_root() {
        return rerun_as_root(&root_args("down", None));
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

/// The arguments `kith` runs itself with under sudo. sudo drops `KITH_SOCKET`, and the
/// root copy would read the socket from the config file, so the socket this run settled
/// on is passed on explicitly.
pub(crate) fn root_args(command: &str, socket: Option<&Path>) -> Vec<OsString> {
    let mut args = Vec::with_capacity(3);
    if let Some(socket) = socket {
        let mut flag = OsString::from("--socket=");
        flag.push(socket);
        args.push(flag);
    }
    args.push(command.into());
    args
}

fn rerun_as_root(args: &[OsString]) -> Result<(), CliError> {
    let me = std::env::current_exe()
        .map_err(|e| CliError(format!("cannot find the kith binary: {e}")))?;
    let status = Command::new("sudo")
        .arg("--")
        .arg(me)
        .args(args)
        .status()
        .map_err(|e| CliError(format!("cannot run sudo: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        let shown: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
        Err(CliError(format!(
            "`sudo kith {}` did not finish",
            shown.join(" ")
        )))
    }
}

/// Writes the service definition at `path` from `template`, pointing it at this binary
/// instead of `placeholder`, unless the same one is there already. A definition from an
/// older release is replaced. Returns whether it wrote one.
pub(crate) fn install_service(
    path: &Path,
    template: &str,
    placeholder: &str,
) -> Result<bool, CliError> {
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
    let wanted = template.replace(placeholder, me);
    if std::fs::read_to_string(path).is_ok_and(|have| have == wanted) {
        return Ok(false);
    }
    std::fs::write(path, wanted)
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
