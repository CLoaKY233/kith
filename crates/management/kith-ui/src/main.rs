//! The `kith-ui` binary: serves the kith page on 127.0.0.1 and opens it in a browser.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use bpaf::Bpaf;
use kith_config::Config;
use kith_ui::{DEFAULT_PORT, Server, Token};

/// Open kith in a web browser. The page stays on this device: it listens on 127.0.0.1 and
/// needs the link printed here.
#[derive(Debug, Clone, Bpaf)]
#[bpaf(options, version)]
struct Opts {
    /// Daemon socket (default: from the config file)
    #[bpaf(long, env("KITH_SOCKET"), argument("PATH"))]
    socket: Option<PathBuf>,
    /// Port to listen on; another free one if it is taken
    #[bpaf(long, argument("PORT"), fallback(DEFAULT_PORT))]
    port: u16,
    /// Print the link instead of opening a browser
    no_open: bool,
}

fn open_browser(url: &str) -> std::io::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
}

async fn run(opts: Opts) -> Result<(), String> {
    let socket = match opts.socket {
        Some(path) => path,
        None => {
            Config::load_or_default(&Config::default_path())
                .map_err(|error| error.to_string())?
                .daemon
                .socket_path
        }
    };
    let listener = kith_ui::bind(opts.port)
        .await
        .map_err(|error| format!("cannot listen on 127.0.0.1: {error}"))?;
    let token = Token::random().map_err(|error| error.to_string())?;
    let server = Server::new(listener, socket, token).map_err(|error| error.to_string())?;
    let url = server.url();
    let mut out = std::io::stdout().lock();
    let _ignored = writeln!(out, "kith ui is at {url}");
    if !opts.no_open && open_browser(&url).is_err() {
        let _ignored = writeln!(out, "Could not open a browser; open the link above.");
    }
    let _ignored = writeln!(out, "Press Ctrl-C to stop.");
    drop(out);
    tokio::select! {
        result = server.run() => result.map_err(|error| format!("stopped serving: {error}")),
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}

fn main() -> ExitCode {
    let opts = opts().run();
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot start runtime: {error}"))
        .and_then(|runtime| runtime.block_on(run(opts)));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ignored = writeln!(std::io::stderr().lock(), "kith-ui: {error}");
            ExitCode::FAILURE
        }
    }
}
