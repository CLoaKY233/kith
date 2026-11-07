//! The `kith-ui` binary: serves the kith page on 127.0.0.1 and opens it in a browser, or,
//! with `--public <domain>`, serves the Console over HTTPS for people with passkeys.

#![forbid(unsafe_code)]

use core::net::{Ipv6Addr, SocketAddr};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use bpaf::Bpaf;
use kith_config::Config as KithConfig;
use kith_ui_core::public::{self, PublicConfig};
use kith_ui_core::{
    Asset, Config, DEFAULT_PORT, Mode, Source, SqliteStore, Storage, Token, bind, load_fixture,
    now_ms, router, setup_link, setup_link_at,
};

/// The built page, or `fallback.html` when it was not built; see `build.rs`.
const ASSETS: &[Asset] = include!(concat!(env!("OUT_DIR"), "/assets.rs"));
/// Vite's dev server, which serves the page under `--dev`.
const VITE: &str = "http://localhost:5173";
/// The database's file name in the state directory.
const DATABASE: &str = "console.sqlite3";
/// Where the public Console writes its setup link, for the installer to print.
const SETUP_LINK: &str = "setup-link";

/// Open kith in a web browser. The page stays on this device: it listens on 127.0.0.1 and
/// needs the link printed here, which works once. With --public, serve the kith Console
/// at a domain over HTTPS instead, for people who sign in with passkeys.
#[derive(Debug, Clone, Bpaf)]
#[bpaf(options, version)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a command-line switch of its own"
)]
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
    /// With --fixture, also print a Console setup link on localhost, to try setup, passkeys
    /// and people against the simulated network. For tests only
    fixture_console: bool,
    /// Serve the Console at this domain over HTTPS (ports 443 and 80), with a certificate
    /// from Let's Encrypt
    #[bpaf(long, argument("DOMAIN"))]
    public: Option<String>,
    /// Where the Console keeps its database and certificate (default: the one systemd gives,
    /// or a folder for kith-ui in this user's state directory)
    #[bpaf(long, argument("DIR"))]
    state_dir: Option<PathBuf>,
    /// Contact email for Let's Encrypt
    #[bpaf(long, env("KITH_CONSOLE_EMAIL"), argument("EMAIL"))]
    acme_email: Option<String>,
    /// Use Let's Encrypt's staging service, whose certificates browsers do not trust
    acme_staging: bool,
    /// HTTPS port in public mode
    #[bpaf(long, argument("PORT"), fallback(443))]
    https_port: u16,
    /// HTTP port in public mode, which only redirects; 0 for none
    #[bpaf(long, argument("PORT"), fallback(80))]
    http_port: u16,
}

/// The state directory: `--state-dir`, systemd's `$STATE_DIRECTORY`, or the user's.
fn state_dir(opts: &Opts) -> Option<PathBuf> {
    if let Some(dir) = &opts.state_dir {
        return Some(dir.clone());
    }
    if let Some(dir) = std::env::var_os("STATE_DIRECTORY") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/kith-ui")
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map_or_else(|| home.join(".local/state"), PathBuf::from)
            .join("kith-ui")
    })
}

/// Opens the database in `dir`, made private to this user.
fn open_store(dir: &Path) -> Result<SqliteStore, String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot protect {}: {error}", dir.display()))?;
    }
    SqliteStore::open(&dir.join(DATABASE)).map_err(|error| error.to_string())
}

/// The database: on disk, or in memory for a fixture or when there is nowhere to keep it.
fn store(opts: &Opts) -> Result<Arc<dyn Storage>, String> {
    let dir = match (&opts.fixture, &opts.state_dir) {
        (Some(_), None) => None,
        _ => state_dir(opts),
    };
    match dir {
        Some(dir) => Ok(Arc::new(open_store(&dir)?)),
        None if opts.public.is_some() => Err("the Console needs --state-dir".to_owned()),
        None => Ok(Arc::new(
            SqliteStore::memory().map_err(|error| error.to_string())?,
        )),
    }
}

fn source(opts: &Opts) -> Result<Source, String> {
    Ok(match (&opts.fixture, &opts.socket) {
        (Some(path), _) => Source::Fixture(Box::new(
            load_fixture(path).map_err(|error| error.to_string())?,
        )),
        (None, Some(socket)) => Source::Daemon(socket.clone()),
        (None, None) => Source::Daemon(
            KithConfig::load_or_default(&KithConfig::default_path())
                .map_err(|error| error.to_string())?
                .daemon
                .socket_path,
        ),
    })
}

/// Serves the Console at `domain` until Ctrl-C or SIGTERM.
async fn run_public(opts: Opts, domain: String) -> Result<(), String> {
    let dir = state_dir(&opts).ok_or("the Console needs --state-dir")?;
    let store: Arc<dyn Storage> = Arc::new(open_store(&dir)?);
    let mut out = std::io::stdout().lock();
    match setup_link(&*store, &domain, now_ms()).map_err(|error| error.to_string())? {
        Some(link) => {
            write_private(&dir.join(SETUP_LINK), &format!("{link}\n"))?;
            let _ignored = writeln!(
                out,
                "Set up the kith Console within 30 minutes; the link works once:\n  {link}"
            );
        }
        None => {
            let _gone = std::fs::remove_file(dir.join(SETUP_LINK));
        }
    }
    let _ignored = writeln!(out, "kith Console at https://{domain}");
    drop(out);
    let app = router(Config {
        source: source(&opts)?,
        port: opts.https_port,
        token: Token::random().map_err(|error| error.to_string())?,
        assets: Some(ASSETS),
        dev_origins: Vec::new(),
        mode: Mode::Public {
            domain: domain.clone(),
        },
        store,
    })
    .map_err(|error| error.to_string())?;
    let config = PublicConfig {
        domain,
        cache_dir: dir.join("acme"),
        contact: opts
            .acme_email
            .filter(|email| !email.trim().is_empty())
            .map(|email| format!("mailto:{}", email.trim())),
        staging: opts.acme_staging,
        https: SocketAddr::from((Ipv6Addr::UNSPECIFIED, opts.https_port)),
        http: (opts.http_port != 0)
            .then(|| SocketAddr::from((Ipv6Addr::UNSPECIFIED, opts.http_port))),
    };
    tokio::select! {
        served = public::serve(app, config) => {
            served.map_err(|error| format!("stopped serving: {error}"))
        }
        () = stopped() => Ok(()),
    }
}

/// Writes `text` to `path`, readable by this user only.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Ctrl-C, or SIGTERM from systemd.
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _interrupted = tokio::signal::ctrl_c() => {}
                    _terminated = term.recv() => {}
                }
            }
            Err(_) => {
                let _interrupted = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _interrupted = tokio::signal::ctrl_c().await;
    }
}

/// Serves until Ctrl-C. Stopping does not wait for connections to close, because the
/// page's event stream never does.
async fn run(opts: Opts) -> Result<(), String> {
    if let Some(domain) = opts.public.clone() {
        return run_public(opts, domain).await;
    }
    if opts.fixture_console && opts.fixture.is_none() {
        return Err("--fixture-console needs --fixture".to_owned());
    }
    let store = store(&opts)?;
    let source = source(&opts)?;
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
    let setup = if opts.fixture_console {
        let origin = if opts.dev {
            VITE.to_owned()
        } else {
            format!("http://localhost:{port}")
        };
        setup_link_at(&*store, &origin, now_ms()).map_err(|error| error.to_string())?
    } else {
        None
    };
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
        mode: Mode::Local,
        store,
    })
    .map_err(|error| error.to_string())?;
    let mut out = std::io::stdout().lock();
    let _ignored = writeln!(out, "kith ui is at {link}");
    if let Some(setup) = setup {
        let _ignored = writeln!(
            out,
            "Set up the simulated Console within 30 minutes; the link works once:\n  {setup}"
        );
    }
    if !opts.no_open && opener::open(&link).is_err() {
        let _ignored = writeln!(out, "Could not open a browser; open the link above.");
    }
    let _ignored = writeln!(out, "Press Ctrl-C to stop.");
    drop(out);
    tokio::select! {
        served = axum::serve(listener, app) => {
            served.map_err(|error| format!("stopped serving: {error}"))
        }
        () = stopped() => Ok(()),
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
