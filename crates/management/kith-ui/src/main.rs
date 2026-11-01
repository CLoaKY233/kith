//! The `kith-ui` binary: serves the kith page on 127.0.0.1 and opens it in a browser.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use bpaf::Bpaf;
use kith_config::Config as KithConfig;
use kith_ui_core::{Asset, Config, DEFAULT_PORT, Source, Token, bind, load_fixture, router};

/// The built page, or `fallback.html` when it was not built; see `build.rs`.
const ASSETS: &[Asset] = include!(concat!(env!("OUT_DIR"), "/assets.rs"));
/// Vite's dev server, which serves the page under `--dev`.
const VITE: &str = "http://localhost:5173";

/// Open kith in a web browser. The page stays on this device: it listens on 127.0.0.1 and
/// needs the link printed here, which works once.
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
    /// Serve only the API, for the page running under `npm run dev`
    dev: bool,
    /// Serve a recorded network instead of talking to the daemon
    #[bpaf(long, argument("FILE"))]
    fixture: Option<PathBuf>,
}

async fn run(opts: Opts) -> Result<(), String> {
    let source = match (opts.fixture, opts.socket) {
        (Some(path), _) => Source::Fixture(Box::new(
            load_fixture(&path).map_err(|error| error.to_string())?,
        )),
        (None, Some(socket)) => Source::Daemon(socket),
        (None, None) => Source::Daemon(
            KithConfig::load_or_default(&KithConfig::default_path())
                .map_err(|error| error.to_string())?
                .daemon
                .socket_path,
        ),
    };
    let listener = bind(opts.port)
        .await
        .map_err(|error| format!("cannot listen on 127.0.0.1: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let token = Token::random().map_err(|error| error.to_string())?;
    let origin = if opts.dev {
        VITE.to_owned()
    } else {
        format!("http://127.0.0.1:{port}")
    };
    let link = format!("{origin}/#t={}", token.hex());
    let app = router(Config {
        source,
        port,
        token,
        assets: (!opts.dev).then_some(ASSETS),
        dev_origins: if opts.dev {
            vec![VITE.to_owned()]
        } else {
            Vec::new()
        },
    })
    .map_err(|error| error.to_string())?;
    let mut out = std::io::stdout().lock();
    let _ignored = writeln!(out, "kith ui is at {link}");
    if !opts.no_open && opener::open(&link).is_err() {
        let _ignored = writeln!(out, "Could not open a browser; open the link above.");
    }
    let _ignored = writeln!(out, "Press Ctrl-C to stop.");
    drop(out);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _interrupted = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|error| format!("stopped serving: {error}"))
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
