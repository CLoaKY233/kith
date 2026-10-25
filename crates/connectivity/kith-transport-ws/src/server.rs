use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::ServerConfig;
use rustls::crypto::ring;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use crate::handshake::{check_path, not_found_response, parse_request, switching_response};
use crate::stream::{Stream, read_head};
use crate::{Error, WsLink};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// A certificate chain and its private key for terminating TLS. Plain data.
#[derive(Debug)]
pub struct TlsIdentity {
    /// End-entity certificate first.
    pub chain: Vec<CertificateDer<'static>>,
    /// Key for the end-entity certificate.
    pub key: PrivateKeyDer<'static>,
}

/// Listens for WebSocket upgrades on one secret path; everything else gets a 404.
#[derive(Debug)]
pub struct WsServer {
    listener: TcpListener,
    path: Arc<str>,
    tls: Option<Arc<ServerConfig>>,
}

impl WsServer {
    /// Binds `address`. Without `tls` it speaks plain HTTP, for use behind a TLS-terminating proxy.
    pub async fn bind(
        address: SocketAddr,
        path: &str,
        tls: Option<TlsIdentity>,
    ) -> Result<Self, Error> {
        check_path(path)?;
        let tls = match tls {
            Some(identity) => {
                let mut config =
                    ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
                        .with_safe_default_protocol_versions()?
                        .with_no_client_auth()
                        .with_single_cert(identity.chain, identity.key)?;
                config.alpn_protocols = vec![b"http/1.1".to_vec()];
                Some(Arc::new(config))
            }
            None => None,
        };
        Ok(Self {
            listener: TcpListener::bind(address).await?,
            path: path.into(),
            tls,
        })
    }

    /// The bound address.
    pub fn local_addr(&self) -> Result<SocketAddr, Error> {
        Ok(self.listener.local_addr()?)
    }

    /// Accepts one TCP connection; run [`Incoming::upgrade`] on its own task.
    pub async fn accept(&self) -> Result<Incoming, Error> {
        let (tcp, peer) = self.listener.accept().await?;
        Ok(Incoming {
            tcp,
            peer,
            path: Arc::clone(&self.path),
            tls: self.tls.clone(),
        })
    }
}

/// An accepted connection that has not yet been upgraded.
#[derive(Debug)]
pub struct Incoming {
    tcp: TcpStream,
    peer: SocketAddr,
    path: Arc<str>,
    tls: Option<Arc<ServerConfig>>,
}

impl Incoming {
    /// The remote address (the proxy's, when behind one).
    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// Completes the upgrade, or answers 404 and returns `None` for any other request.
    pub async fn upgrade(self) -> Result<Option<WsLink>, Error> {
        tokio::time::timeout(HANDSHAKE_TIMEOUT, self.upgrade_inner())
            .await
            .map_err(|_| Error::Timeout)?
    }

    async fn upgrade_inner(self) -> Result<Option<WsLink>, Error> {
        self.tcp.set_nodelay(true)?;
        let mut stream = match self.tls {
            Some(config) => Stream::Tls(Box::new(
                TlsAcceptor::from(config).accept(self.tcp).await?.into(),
            )),
            None => Stream::Plain(self.tcp),
        };
        let head = read_head(&mut stream).await?;
        let parsed = parse_request(&head);
        if let Ok(Some((request, used))) = &parsed
            && request.path == &*self.path
            && let Some(key) = request.upgrade_key
        {
            stream.write_all(switching_response(key).as_bytes()).await?;
            stream.flush().await?;
            return WsLink::new(stream, false, head.get(*used..).unwrap_or_default()).map(Some);
        }
        stream.write_all(not_found_response().as_bytes()).await?;
        stream.shutdown().await?;
        Ok(None)
    }
}
