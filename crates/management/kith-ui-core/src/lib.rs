//! The kith page's server: an axum [`Router`] that `kith-ui` serves on `127.0.0.1` and the
//! desktop shell mounts on loopback.
//!
//! Plane: management. See `docs/plans/kith-ui-api.md` for the HTTP contract and
//! `docs/plans/kith-ui-design.md` §1–2 for why it is shaped this way.
//!
//! The page reaches the daemon only through this router. Every request must name this
//! machine in `Host`, every change must come from the page's own origin, and everything
//! but the page shell needs the session cookie that the one-time link starts.

#![forbid(unsafe_code)]

mod api;
mod assets;
mod fixture;
mod ipc;
pub mod json;
pub mod osauth;
mod session;
mod stream;
pub mod traffic;
#[cfg(feature = "ts")]
pub mod ts;
pub mod wire;

use core::net::Ipv4Addr;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use tokio::net::TcpListener;

pub use assets::Asset;
pub use fixture::{FixtureError, load_fixture};
pub use session::Token;
pub use wire::Fixture;

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
    /// The port the router is served on, for the `Host` and `Origin` checks.
    pub port: u16,
    /// The one-time token in the link that `kith ui` opens.
    pub token: Token,
    /// The page's files, or `None` in dev mode, where Vite serves them.
    pub assets: Option<&'static [Asset]>,
    /// Origins allowed besides this server's own, such as Vite's in dev mode.
    pub dev_origins: Vec<String>,
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
}

/// The whole server: the API, the event stream and the page.
///
/// Call it inside a tokio runtime: it starts the task that follows the daemon or replays
/// the fixture.
pub fn router(config: Config) -> io::Result<Router> {
    let boot = match &config.source {
        Source::Fixture(fixture) => fixture.boot.clone(),
        Source::Daemon(_) => Token::random()?
            .hex()
            .get(..16)
            .unwrap_or_default()
            .to_owned(),
    };
    let source = ipc::Source::new(config.source);
    let live = Arc::new(stream::Live::new(&boot));
    let follower = ipc::follow(&source, Arc::clone(&live))?;
    let shared = Arc::new(Shared {
        source,
        sessions: session::Sessions::new(config.token, config.port),
        port: config.port,
        dev_origins: config.dev_origins,
        assets: config.assets,
        boot,
        live,
        _follower: follower,
    });
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
        .route("/phones/{node}/config", post(api::phones::config))
        .route("/joins/{node}/approve", post(api::joins::approve))
        .route("/joins/{node}/deny", post(api::joins::deny))
        .route("/devices/{node}/rename", post(api::devices::rename))
        .route("/devices/{node}/remove", post(api::devices::remove))
        .route("/leave", post(api::devices::leave));
    let routes = Router::new()
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
        .nest("/api/networks/{net}", networks)
        .fallback(assets::serve);
    Ok(routes
        .layer(from_fn_with_state(Arc::clone(&shared), session::guard))
        .with_state(shared))
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
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
