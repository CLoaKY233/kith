//! The `kith-ui` server: a web page on `127.0.0.1` that shows and changes this device's kith.
//!
//! Plane: management. Binary. See `docs/plans/kith-ui-api.md` for the HTTP contract.
//!
//! The page reaches the daemon only through this process, which holds a random session
//! token. A browser gets the token once through the URL that `kith-ui` opens and keeps it in
//! an `HttpOnly` cookie; other local pages cannot read it, and `POST`s from other origins are
//! refused.

#![forbid(unsafe_code)]

mod api;
mod assets;
mod events;
mod json;
mod session;

use core::convert::Infallible;
use core::net::{Ipv4Addr, SocketAddr};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

pub use session::Token;

/// The port tried first.
pub const DEFAULT_PORT: u16 = 41680;

type Body = BoxBody<Bytes, Infallible>;

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

/// A running page server for one daemon.
#[derive(Debug)]
pub struct Server {
    listener: TcpListener,
    state: Arc<State>,
}

#[derive(Debug)]
struct State {
    socket: PathBuf,
    token: Token,
    origin: String,
}

impl Server {
    /// Serves on `listener`, talking to the daemon at `socket`, with session `token`.
    pub fn new(listener: TcpListener, socket: PathBuf, token: Token) -> io::Result<Self> {
        let address = listener.local_addr()?;
        Ok(Self {
            listener,
            state: Arc::new(State {
                socket,
                token,
                origin: format!("http://{address}"),
            }),
        })
    }

    /// Where the server listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The URL that starts a session: open it in a browser.
    pub fn url(&self) -> String {
        format!("{}/?token={}", self.state.origin, self.state.token.hex())
    }

    /// Accepts connections until accepting fails.
    pub async fn run(self) -> io::Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            let state = Arc::clone(&self.state);
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let state = Arc::clone(&state);
                    async move { Ok::<_, Infallible>(session::handle(&state, request).await) }
                });
                // A client hanging up mid-request is its own business.
                let _ignored = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    }
}
