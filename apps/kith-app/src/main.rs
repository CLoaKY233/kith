//! The kith desktop shell (design spec §15 M2): a native tray menu with kith's state, the
//! exit choice and what needs this person; native notifications whose only action is
//! `Review`; starting at login, offered once; `kith://join/` links; and a window onto the
//! kith page, served by `kith-ui-core` on loopback while the window is open and destroyed
//! with it.
//!
//! The shell never talks to the daemon except through `kith-ui-core`, so it shows exactly
//! what the page shows. `--fixture` runs it against a recorded network, without a daemon.

#![forbid(unsafe_code)]

mod autostart;
mod client;
mod copy;
mod deeplink;
mod icon;
mod model;
mod notify;
mod page;
mod shell;
mod sse;
mod timing;
mod tokens;
mod watch;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use bpaf::Bpaf;
use kith_config::Config as KithConfig;
use kith_ui_core::load_fixture;
use tauri_plugin_autostart::MacosLauncher;

use crate::shell::Origin;

/// kith in the menu bar.
#[derive(Debug, Clone, Bpaf)]
#[bpaf(options, version)]
struct Opts {
    /// Daemon socket (default: from the config file)
    #[bpaf(long, env("KITH_SOCKET"), argument("PATH"))]
    socket: Option<PathBuf>,
    /// Show a recorded network instead of talking to the daemon
    #[bpaf(long, argument("FILE"))]
    fixture: Option<PathBuf>,
    /// Links the OS passes when it opens a kith:// link; the deep-link plugin reads them
    #[bpaf(positional("LINK"))]
    #[expect(
        dead_code,
        reason = "accepted so the OS's arguments parse; the plugin reads them"
    )]
    links: Vec<String>,
}

fn origin(opts: Opts) -> Result<Origin, String> {
    Ok(match (opts.fixture, opts.socket) {
        (Some(path), _) => Origin::Fixture(Arc::new(
            load_fixture(&path).map_err(|error| error.to_string())?,
        )),
        (None, Some(socket)) => Origin::Daemon(socket),
        (None, None) => Origin::Daemon(
            KithConfig::load_or_default(&KithConfig::default_path())
                .map_err(|error| error.to_string())?
                .daemon
                .socket_path,
        ),
    })
}

fn run(origin: Origin) -> Result<(), String> {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            shell::second_launch(app, &argv);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| Ok(shell::setup(app, origin)?))
        .build(tauri::generate_context!())
        .map_err(|error| error.to_string())?;
    app.run(shell::on_event);
    Ok(())
}

fn main() -> ExitCode {
    match origin(opts().run()).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ignored = writeln!(std::io::stderr().lock(), "kith-app: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use kith_ui_core::wire::Fixture;

    use crate::model::{BootIn, Snapshot};

    /// The recorded office network the page is built against.
    pub(crate) fn office() -> Fixture {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/management/kith-ui/web/fixtures/office.json");
        kith_ui_core::load_fixture(&path).expect("the office fixture loads")
    }

    /// The office network as the shell reads it, without a router.
    pub(crate) fn office_snapshot() -> Snapshot {
        let fixture = office();
        let boot = serde_json::json!({
            "boot": fixture.boot,
            "seq": 230,
            "server_time_ms": fixture.recorded_at_ms,
            "status": fixture.status,
            "peers": fixture.peers,
            "pending": fixture.pending,
        });
        let boot: BootIn = serde_json::from_value(boot).expect("the snapshot reads");
        Snapshot {
            boot,
            exits: fixture.exits.into_iter().collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn the_shell_ships_with_the_workspace_version() {
        let root = include_str!("../../../Cargo.toml");
        let line = root
            .lines()
            .find(|line| line.starts_with("version = "))
            .expect("the workspace has a version");
        assert_eq!(line, format!("version = \"{}\"", env!("CARGO_PKG_VERSION")));
    }
}
