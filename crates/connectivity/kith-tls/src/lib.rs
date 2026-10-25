//! TLS 1.3 configuration authenticated by kith identities.
//!
//! Raw public keys (RFC 7250), no CA. Each side presents its Ed25519 key as a bare
//! `SubjectPublicKeyInfo`; the client pins the server to an expected [`NodeId`] and the server
//! accepts any valid key, leaving membership to the caller via [`peer_node_id`].

#![forbid(unsafe_code)]

use std::sync::Arc;

use kith_crypto::{SigningKey, VerifyingKey};
use kith_types::NodeId;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::sign::CertifiedKey;

mod der;
mod verify;

/// Building a TLS configuration failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// rustls rejected the configuration or key.
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    /// The expected server identity is not a valid Ed25519 key.
    #[error("key: {0}")]
    Key(#[from] kith_crypto::SignatureError),
}

/// Server config: presents `identity`, requires and accepts any Ed25519 client key.
pub fn server_config(
    identity: &SigningKey,
    alpn: &[&[u8]],
) -> Result<Arc<rustls::ServerConfig>, Error> {
    let provider = provider();
    let verifier = Arc::new(verify::AnyRawKeyClient {
        provider: Arc::clone(&provider),
    });
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_cert_resolver(Arc::new(
            rustls::server::AlwaysResolvesServerRawPublicKeys::new(certified_key(identity)?),
        ));
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(Arc::new(config))
}

/// Client config: presents `identity`, accepts only `expected_server`'s key.
pub fn client_config(
    identity: &SigningKey,
    expected_server: NodeId,
    alpn: &[&[u8]],
) -> Result<Arc<rustls::ClientConfig>, Error> {
    let provider = provider();
    let expected_spki = der::spki(&VerifyingKey::from_node_id(expected_server)?.to_bytes());
    let verifier = Arc::new(verify::PinnedRawKeyServer {
        expected_spki,
        provider: Arc::clone(&provider),
    });
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_cert_resolver(Arc::new(
            rustls::client::AlwaysResolvesClientRawPublicKeys::new(certified_key(identity)?),
        ));
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(Arc::new(config))
}

/// The authenticated peer identity, from the peer's certificates (`CommonState::peer_certificates`).
#[must_use]
pub fn peer_node_id(peer_certificates: &[CertificateDer<'_>]) -> Option<NodeId> {
    match peer_certificates {
        [spki] => der::node_id_from_spki(spki.as_ref()),
        _ => None,
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn certified_key(identity: &SigningKey) -> Result<Arc<CertifiedKey>, Error> {
    let pkcs8 = der::pkcs8(&identity.to_seed());
    let key =
        rustls::crypto::ring::sign::any_eddsa_type(&PrivatePkcs8KeyDer::from(pkcs8.as_ref()))?;
    let cert = CertificateDer::from(der::spki(&identity.verifying_key().to_bytes()).to_vec());
    Ok(Arc::new(CertifiedKey::new(vec![cert], key)))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use kith_crypto::SigningKey;
    use kith_types::NodeId;
    use rustls::pki_types::ServerName;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    use super::{client_config, peer_node_id, server_config};

    const ALPN: &[&[u8]] = &[b"kith/1"];

    /// Runs a handshake over loopback TCP; returns what each side saw of the other.
    async fn handshake(expected: NodeId) -> (Option<NodeId>, Option<NodeId>) {
        let server_key = SigningKey::from_seed(&[1; 32]);
        let client_key = SigningKey::from_seed(&[2; 32]);
        let acceptor = TlsAcceptor::from(server_config(&server_key, ALPN).unwrap());
        let connector = TlsConnector::from(client_config(&client_key, expected, ALPN).unwrap());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut tls = acceptor.accept(tcp).await.ok()?;
            let mut byte = [0_u8; 1];
            tls.read_exact(&mut byte).await.ok()?;
            tls.get_ref().1.peer_certificates().and_then(peer_node_id)
        });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let name = ServerName::try_from("kith.invalid").unwrap();
        let client_saw = match connector.connect(name, tcp).await {
            Ok(mut tls) => {
                tls.write_all(&[0]).await.unwrap();
                tls.flush().await.unwrap();
                tls.get_ref().1.peer_certificates().and_then(peer_node_id)
            }
            Err(_) => None,
        };
        (client_saw, server.await.unwrap())
    }

    #[tokio::test]
    async fn mutual_auth_exposes_both_identities() {
        let server = SigningKey::from_seed(&[1; 32]).node_id();
        let client = SigningKey::from_seed(&[2; 32]).node_id();
        assert_eq!(handshake(server).await, (Some(server), Some(client)));
    }

    #[tokio::test]
    async fn client_pinned_to_wrong_server_fails() {
        let other = SigningKey::from_seed(&[9; 32]).node_id();
        assert_eq!(handshake(other).await, (None, None));
    }
}
