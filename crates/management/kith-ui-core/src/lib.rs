//! The kith page's server: an axum [`Router`] that `kith-ui` serves on `127.0.0.1` and the
//! desktop shell mounts on loopback, or, as the Console, over HTTPS at a public domain.
//!
//! Plane: management. See `docs/plans/kith-ui-api.md` for the HTTP contract,
//! `docs/plans/kith-ui-design.md` §1–2 for why it is shaped this way, and
//! `docs/plans/console.md` for people, passkeys and public mode.
//!
//! The page reaches the daemon only through this router. Every request must name this
//! server in `Host`, every change must come from the page's own origin, and everything
//! but the page shell needs a session: the one started by the local one-time link, or a
//! person's, started with a passkey.

#![forbid(unsafe_code)]

mod accounts;
mod api;
mod assets;
mod console;
mod fixture;
mod ipc;
pub mod json;
pub mod osauth;
pub mod public;
mod session;
mod sim;
pub mod store;
mod stream;
pub mod traffic;
#[cfg(feature = "ts")]
pub mod ts;
mod webauthn;
pub mod wire;

use core::net::Ipv4Addr;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

pub use assets::Asset;
pub use console::{Mode, SETUP_MS};
pub use fixture::{FixtureError, load_fixture};
pub use session::Token;
pub use store::{SqliteStore, Storage, StoreError};
pub use wire::Fixture;

use crate::accounts::{AccountsBackend, IpcAccounts, MemoryAccounts};

/// The port tried first.
pub const DEFAULT_PORT: u16 = 41680;

/// Where the page's data comes from.
#[derive(Debug)]
pub enum Source {
    /// The daemon on this Unix socket.
    Daemon(PathBuf),
    /// A recorded network, for building the page without a daemon.
    Fixture(Box<Fixture>),
}

/// Everything [`router`] needs.
#[derive(Debug)]
pub struct Config {
    /// Where the data comes from.
    pub source: Source,
    /// The port the router is served on, for the `Host` and `Origin` checks in local mode.
    pub port: u16,
    /// The one-time token in the link that `kith ui` opens; unused in public mode.
    pub token: Token,
    /// The page's files, or `None` in dev mode, where Vite serves them.
    pub assets: Option<&'static [Asset]>,
    /// Origins allowed besides this server's own, such as Vite's in dev mode.
    pub dev_origins: Vec<String>,
    /// Local, or public at a domain.
    pub mode: Mode,
    /// The Console's database: sessions, passkey prompts, codes and history.
    pub store: Arc<dyn Storage>,
}

/// State every handler shares.
#[derive(Debug)]
struct Shared {
    source: ipc::Source,
    sessions: session::Sessions,
    port: u16,
    dev_origins: Vec<String>,
    assets: Option<&'static [Asset]>,
    /// The boot id of the first run. The current one is [`stream::Live::position`], which
    /// changes when the daemon restarts.
    boot: String,
    /// The journal, ticks and traffic history that every stream and snapshot reads.
    live: Arc<stream::Live>,
    /// Keeps `live` current; stops when the router is dropped.
    _follower: ipc::Follower,
    /// What the write APIs keep between requests, and the tier-3 gate.
    api: api::State,
    /// Local or public.
    mode: Mode,
    /// The Console's database.
    store: Arc<dyn Storage>,
    /// People in the ledger.
    accounts: Arc<dyn AccountsBackend>,
    /// The time, for everything that lapses.
    clock: Arc<console::Clock>,
    /// Tries of the public code lookup per address.
    limiter: console::RateLimiter,
    /// Writes history to `store`; stops when the router is dropped.
    _worker: console::Worker,
}

/// The whole server: the API, the event stream and the page.
///
/// Call it inside a tokio runtime: it starts the tasks that follow the daemon or replay
/// the fixture, and that keep history in the database.
pub fn router(config: Config) -> io::Result<Router> {
    build(config, osauth::Gate::System, None).map(|(router, _)| router)
}

/// A new setup link for the public Console at `domain`, valid [`SETUP_MS`] from `now_ms`;
/// every earlier unused link stops working. `None` once the first owner exists.
pub fn setup_link(
    store: &dyn Storage,
    domain: &str,
    now_ms: u64,
) -> Result<Option<String>, StoreError> {
    setup_link_at(store, &format!("https://{domain}"), now_ms)
}

/// [`setup_link`] for a page at `origin`, such as `http://localhost:41680` for a local
/// server, where passkeys work only on `localhost`.
pub fn setup_link_at(
    store: &dyn Storage,
    origin: &str,
    now_ms: u64,
) -> Result<Option<String>, StoreError> {
    if store.setting(console::SETUP_DONE_KEY)?.is_some() {
        return Ok(None);
    }
    let token = Token::random()
        .map_err(|error| StoreError::new(error.to_string()))?
        .hex();
    store.put_setup_token(
        &store::secret_id(&token),
        now_ms,
        now_ms.saturating_add(SETUP_MS),
    )?;
    Ok(Some(format!("{origin}/setup/{token}")))
}

/// [`router`], with tier-3 changes confirmed by `gate` and, when given, people kept by
/// `accounts` instead of the source's own.
fn build(
    config: Config,
    gate: osauth::Gate,
    accounts: Option<Arc<dyn AccountsBackend>>,
) -> io::Result<(Router, Arc<Shared>)> {
    let boot = match &config.source {
        Source::Fixture(fixture) => fixture.boot.clone(),
        Source::Daemon(_) => Token::random()?
            .hex()
            .get(..16)
            .unwrap_or_default()
            .to_owned(),
    };
    let source = ipc::Source::new(config.source);
    let accounts = accounts.unwrap_or_else(|| match &source {
        ipc::Source::Daemon(daemon) => Arc::new(IpcAccounts::new(daemon.clone())),
        ipc::Source::Fixture(_) => Arc::new(MemoryAccounts::default()),
    });
    let clock = Arc::new(console::Clock::default());
    let (sink, queue) = mpsc::channel(console::PERSIST_QUEUE);
    let worker = console::start_worker(
        Arc::clone(&config.store),
        Arc::clone(&accounts),
        Arc::clone(&clock),
        queue,
    )?;
    let live = Arc::new(stream::Live::with_sink(&boot, Some(sink)));
    let follower = ipc::follow(&source, Arc::clone(&live))?;
    let sessions = match config.mode {
        Mode::Local => session::Sessions::new(config.token, config.port),
        Mode::Public { .. } => session::Sessions::public(),
    };
    let shared = Arc::new(Shared {
        source,
        sessions,
        port: config.port,
        dev_origins: config.dev_origins,
        assets: config.assets,
        boot,
        live,
        _follower: follower,
        api: api::State::new(gate),
        mode: config.mode,
        store: config.store,
        accounts,
        clock,
        limiter: console::RateLimiter::default(),
        _worker: worker,
    });
    let app = routes()
        .layer(from_fn_with_state(Arc::clone(&shared), session::guard))
        .with_state(Arc::clone(&shared));
    Ok((app, shared))
}

/// Every route, before the guard and the state.
fn routes() -> Router<Arc<Shared>> {
    let networks = Router::new()
        .route("/peers", get(api::peers::list))
        .route("/records", get(api::records::list).post(api::records::set))
        .route("/records/{name}/delete", post(api::records::delete))
        .route("/exits", get(api::exits::list))
        .route("/exits/{node}", post(api::exits::grant))
        .route(
            "/invites",
            get(api::invites::list).post(api::invites::create),
        )
        .route("/invites/{id}/revoke", post(api::invites::revoke))
        .route("/phones", post(api::phones::create))
        .route("/phones/gateways", get(api::phones::gateways))
        .route("/phones/{node}/config", post(api::phones::config))
        .route("/joins/{node}/approve", post(api::joins::approve))
        .route("/joins/{node}/deny", post(api::joins::deny))
        .route("/request-code", get(api::joins::request_code))
        .route("/devices/{node}/rename", post(api::devices::rename))
        .route("/devices/{node}/remove", post(api::devices::remove))
        .route("/leave", post(api::devices::leave))
        .route("/policy", get(api::policy::get).post(api::policy::set))
        .route("/relays", get(api::relays::list).post(api::relays::add))
        .route("/relays/{name}/remove", post(api::relays::remove));
    let console = Router::new()
        .route("/api/me", get(api::auth::me))
        .route("/api/logout", post(api::auth::logout))
        .route("/api/setup/{token}", get(api::auth::setup_info))
        .route("/api/setup/begin", post(api::auth::setup_begin))
        .route("/api/setup/finish", post(api::auth::setup_finish))
        .route("/api/setup/recovery", post(api::auth::recovery))
        .route("/api/login/begin", post(api::auth::login_begin))
        .route("/api/login/finish", post(api::auth::login_finish))
        .route("/api/register/{token}", get(api::auth::register_info))
        .route("/api/register/begin", post(api::auth::register_begin))
        .route("/api/register/finish", post(api::auth::register_finish))
        .route("/api/recover/begin", post(api::auth::recover_begin))
        .route("/api/recover/finish", post(api::auth::recover_finish))
        .route(
            "/api/people",
            get(api::people::list).post(api::people::invite),
        )
        .route("/api/people/{id}/role", post(api::people::role))
        .route("/api/people/{id}/remove", post(api::people::remove))
        .route("/api/people/{id}/approve", post(api::people::approve))
        .route("/api/authority/begin", post(api::people::authority_begin))
        .route("/api/authority/finish", post(api::people::authority_finish))
        .route("/api/passkeys/begin", post(api::people::passkey_begin))
        .route("/api/passkeys/finish", post(api::people::passkey_finish))
        .route("/api/device-codes", post(api::devicecodes::create))
        .route("/api/history/journal", get(api::history::journal))
        .route("/api/history/traffic", get(api::history::traffic))
        .route("/j/{code}", get(api::devicecodes::redeem));
    Router::new()
        .route("/api/session", post(session::start))
        .route("/api/boot", get(api::status::boot))
        .route("/api/status", get(api::status::status))
        .route("/api/pending", get(api::status::pending))
        .route("/api/traffic", get(api::traffic::traffic))
        .route("/api/events", get(stream::events))
        .route("/api/exit", post(api::exits::use_exit))
        .route("/api/networks", post(api::joins::create_network))
        .route("/api/join", post(api::joins::join))
        .route("/api/join/request", post(api::joins::request))
        .route("/api/blocked/{id}/allow", post(api::blocked::allow))
        .route("/api/validate/name", get(api::validate::name))
        .route("/api/server", get(api::server::get).post(api::server::set))
        .route("/api/wireguard/import", post(api::server::import))
        .route("/api/wireguard/{net}/use", post(api::server::use_import))
        .route(
            "/api/wireguard/{net}/remove",
            post(api::server::remove_import),
        )
        .nest("/api/networks/{net}", networks)
        .merge(console)
        .fallback(assets::serve)
}

/// Binds `127.0.0.1:port`, or any free port when that one is taken.
pub async fn bind(port: u16) -> io::Result<TcpListener> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await
        }
        Err(error) => Err(error),
    }
}

/// Milliseconds since the Unix epoch, by this machine's clock.
#[expect(
    clippy::disallowed_methods,
    reason = "the page server is a driver: snapshots and ticks carry the wall-clock time"
)]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
