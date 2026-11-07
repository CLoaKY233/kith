//! Serving the public Console: HTTPS with a Let's Encrypt certificate, obtained and
//! renewed by ACME's TLS-ALPN-01 challenge on the same port, and plain HTTP that only
//! redirects to HTTPS.
//!
//! The certificate and the ACME account are cached in a directory, so a restart does not
//! ask Let's Encrypt again.

use core::net::SocketAddr;
use core::pin::Pin;
use core::time::Duration;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::extract::connect_info::Connected;
use axum::http::header::{self, HeaderValue};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::serve::{IncomingStream, Listener};
use futures_core::Stream;
use rustls::ServerConfig;
use rustls::server::Acceptor;
use rustls_acme::caches::DirCache;
use rustls_acme::{AcmeConfig, is_tls_alpn_challenge};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::LazyConfigAcceptor;
use tokio_rustls::server::TlsStream;

/// How long a client may take to finish the TLS handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// Finished handshakes waiting for the server to take them.
const BACKLOG: usize = 128;

/// Where and how the public Console is served.
#[derive(Debug, Clone)]
pub struct PublicConfig {
    /// The domain, such as `vpn.example.com`; its DNS must point at this machine.
    pub domain: String,
    /// Where the certificate and ACME account are cached.
    pub cache_dir: PathBuf,
    /// A contact address for Let's Encrypt, such as `mailto:ops@example.com`.
    pub contact: Option<String>,
    /// Let's Encrypt's staging service, for trying things out without its rate limits.
    pub staging: bool,
    /// Where HTTPS listens, normally port 443.
    pub https: SocketAddr,
    /// Where the HTTP redirect listens, normally port 80; `None` for none.
    pub http: Option<SocketAddr>,
}

/// A TLS connection, ready for HTTP.
type Tls = TlsStream<TcpStream>;

/// The address a request came from, as `ConnectInfo<ClientAddr>`; the public code lookup
/// counts tries per address with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddr(pub SocketAddr);

impl Connected<IncomingStream<'_, TlsListener>> for ClientAddr {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

impl Connected<IncomingStream<'_, TcpListener>> for ClientAddr {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

/// Connections whose TLS handshake finished, for `axum::serve`.
#[derive(Debug)]
pub struct TlsListener {
    ready: mpsc::Receiver<(Tls, SocketAddr)>,
    local: SocketAddr,
}

impl Listener for TlsListener {
    type Io = Tls;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(connection) => connection,
            // The accept loop stopped; there will be no more connections.
            None => core::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// Serves `app` over HTTPS at `config.domain`, and the redirect over HTTP, until either
/// stops. Must run inside a tokio runtime; binding ports below 1024 needs
/// `CAP_NET_BIND_SERVICE` on Linux.
pub async fn serve(app: Router, config: PublicConfig) -> io::Result<()> {
    let listener = TcpListener::bind(config.https).await?;
    let tls = tls_listener(listener, &config)?;
    let https = axum::serve(tls, app.into_make_service_with_connect_info::<ClientAddr>());
    match config.http {
        Some(address) => {
            let plain = TcpListener::bind(address).await?;
            let redirect = axum::serve(plain, redirect(config.domain.clone()));
            tokio::select! {
                served = https => served,
                served = redirect => served,
            }
        }
        None => https.await,
    }
}

/// Starts ACME for the domain and the loop that accepts and handshakes connections.
fn tls_listener(listener: TcpListener, config: &PublicConfig) -> io::Result<TlsListener> {
    let local = listener.local_addr()?;
    let mut state = AcmeConfig::new([config.domain.clone()])
        .contact(config.contact.iter().cloned())
        .cache(DirCache::new(config.cache_dir.clone()))
        .directory_lets_encrypt(!config.staging)
        .state();
    let challenge = state.challenge_rustls_config();
    let mut serving =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(io::Error::other)?
            .with_no_client_auth()
            .with_cert_resolver(state.resolver());
    serving.alpn_protocols = vec![b"http/1.1".to_vec()];
    let serving = Arc::new(serving);
    tokio::spawn(async move {
        // Orders and renews the certificate; each event is one step or one failure,
        // retried by the state itself.
        while let Some(_event) =
            core::future::poll_fn(|cx| Pin::new(&mut state).poll_next(cx)).await
        {}
    });
    let (sender, ready) = mpsc::channel(BACKLOG);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, peer)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            let sender = sender.clone();
            let challenge = Arc::clone(&challenge);
            let serving = Arc::clone(&serving);
            tokio::spawn(async move {
                let handshake = async {
                    let start = LazyConfigAcceptor::new(Acceptor::default(), tcp)
                        .await
                        .ok()?;
                    if is_tls_alpn_challenge(&start.client_hello()) {
                        let _validated = start.into_stream(challenge).await;
                        return None;
                    }
                    start.into_stream(serving).await.ok()
                };
                if let Ok(Some(tls)) = tokio::time::timeout(HANDSHAKE, handshake).await {
                    let _gone = sender.send((tls, peer)).await;
                }
            });
        }
    });
    Ok(TlsListener { ready, local })
}

/// The plain-HTTP server: every request goes to the same path on `https://<domain>`.
pub fn redirect(domain: String) -> Router {
    let domain = Arc::new(domain);
    Router::new().fallback(move |request: Request| {
        let domain = Arc::clone(&domain);
        async move { to_https(&domain, request.uri()) }
    })
}

fn to_https(domain: &str, uri: &Uri) -> Response {
    let path = uri.path_and_query().map_or("/", |path| path.as_str());
    let target = format!("https://{domain}{path}");
    match HeaderValue::from_str(&target) {
        Ok(location) => {
            let mut response = StatusCode::PERMANENT_REDIRECT.into_response();
            response.headers_mut().insert(header::LOCATION, location);
            response
        }
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use axum::body::Body;
    use tower::ServiceExt as _;

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn plain_http_goes_to_https_on_the_domain() -> Result<(), Box<dyn core::error::Error>> {
        let response = redirect("vpn.example.com".to_owned())
            .oneshot(
                Request::get("/j/ABC123?x=1")
                    .header(header::HOST, "evil.example.com")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            response.headers().get(header::LOCATION),
            Some(&HeaderValue::from_static(
                "https://vpn.example.com/j/ABC123?x=1"
            ))
        );
        Ok(())
    }
}
