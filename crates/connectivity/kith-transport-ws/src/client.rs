use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, ring};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::handshake::{client_request, key_from_nonce, parse_response};
use crate::stream::{Stream, read_head};
use crate::{Error, WsLink};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// How a client connects. Plain data.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// `Host` header and TLS SNI; defaults to the URL's host. Set it to dial a CDN edge by IP.
    pub server_name: Option<String>,
    /// Accept exactly this end-entity certificate instead of `WebPKI` validation (direct-to-origin).
    pub pinned_certificate: Option<CertificateDer<'static>>,
    /// `User-Agent` header.
    pub user_agent: String,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            server_name: None,
            pinned_certificate: None,
            user_agent: concat!("kith/", env!("CARGO_PKG_VERSION")).to_owned(),
        }
    }
}

struct Target<'a> {
    host: &'a str,
    port: u16,
    path: &'a str,
}

fn parse_url(url: &str) -> Result<Target<'_>, Error> {
    let rest = url
        .strip_prefix("wss://")
        .ok_or(Error::InvalidUrl("scheme must be wss"))?;
    let (authority, path) = match rest.find('/') {
        Some(slash) => rest.split_at(slash),
        None => (rest, "/"),
    };
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, after) = bracketed
            .split_once(']')
            .ok_or(Error::InvalidUrl("unclosed ipv6 literal"))?;
        (host, after.strip_prefix(':'))
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let port = port
        .map_or(Ok(443), str::parse)
        .map_err(|_| Error::InvalidUrl("bad port"))?;
    if host.is_empty() {
        return Err(Error::InvalidUrl("missing host"));
    }
    Ok(Target { host, port, path })
}

#[derive(Debug)]
struct PinnedCertificate {
    certificate: CertificateDer<'static>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.certificate.as_ref() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn tls_config(pinned: Option<&CertificateDer<'static>>) -> Result<ClientConfig, Error> {
    let provider = Arc::new(ring::default_provider());
    let algorithms = provider.signature_verification_algorithms;
    let builder =
        ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions()?;
    let mut config = if let Some(certificate) = pinned {
        let verifier = PinnedCertificate {
            certificate: certificate.clone(),
            algorithms,
        };
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth()
    } else {
        let roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// Connects to `wss://host[:port]/path` and completes the WebSocket upgrade.
pub async fn connect(url: &str, options: &ClientOptions) -> Result<WsLink, Error> {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, connect_inner(url, options))
        .await
        .map_err(|_| Error::Timeout)?
}

async fn connect_inner(url: &str, options: &ClientOptions) -> Result<WsLink, Error> {
    let target = parse_url(url)?;
    let name = options.server_name.as_deref().unwrap_or(target.host);
    let host_header = if options.server_name.is_some() || target.port == 443 {
        name.to_owned()
    } else if target.host.contains(':') {
        format!("[{name}]:{}", target.port)
    } else {
        format!("{name}:{}", target.port)
    };
    let mut nonce = [0u8; 16];
    kith_crypto::fill_random(&mut nonce)?;
    let key = key_from_nonce(&nonce);
    let request = client_request(&host_header, target.path, &options.user_agent, &key)?;
    let server_name =
        ServerName::try_from(name.to_owned()).map_err(|_| Error::InvalidField("server name"))?;

    let tcp = TcpStream::connect((target.host, target.port)).await?;
    tcp.set_nodelay(true)?;
    let connector = TlsConnector::from(Arc::new(tls_config(options.pinned_certificate.as_ref())?));
    let tls = connector.connect(server_name, tcp).await?;
    let mut stream = Stream::Tls(Box::new(tls.into()));

    stream.write_all(request.as_bytes()).await?;
    stream.flush().await?;
    let head = read_head(&mut stream).await?;
    let used = parse_response(&head, &key)?.ok_or(Error::Handshake("incomplete response"))?;
    WsLink::new(stream, true, head.get(used..).unwrap_or_default())
}
