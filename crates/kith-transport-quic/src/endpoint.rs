//! The QUIC endpoint and per-peer link.

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use kith_crypto::SigningKey;
use kith_types::NodeId;
use quinn::crypto::rustls::{HandshakeData, QuicClientConfig, QuicServerConfig};
use quinn::{Connection, TransportConfig, VarInt};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::CertificateDer;
use rustls::sign::CertifiedKey;

use crate::{
    DATAGRAM_RECEIVE_BUFFER, DATAGRAM_SEND_BUFFER, KEEP_ALIVE_INTERVAL, MAX_IDLE_TIMEOUT,
    QuicError, auth, der,
};

/// Application close code for an orderly shutdown.
const CLOSE_OK: VarInt = VarInt::from_u32(0);

/// A QUIC-on-443 endpoint that both dials and accepts kith peers.
#[derive(Debug)]
pub struct QuicEndpoint {
    endpoint: quinn::Endpoint,
    identity: Arc<CertifiedKey>,
    provider: Arc<CryptoProvider>,
    transport: Arc<TransportConfig>,
    node_id: NodeId,
}

impl QuicEndpoint {
    /// Binds a UDP socket and derives the local identity from `identity_seed`.
    pub fn bind(addr: SocketAddr, identity_seed: &[u8; 32]) -> Result<Self, QuicError> {
        let signing = SigningKey::from_seed(identity_seed);
        let node_id = signing.node_id();
        let public_key = signing.verifying_key().to_bytes();

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let identity = auth::certified_key(identity_seed, &public_key)?;
        let transport = transport_config()?;

        let server_crypto = auth::server_config(&provider, Arc::clone(&identity))?;
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
            QuicServerConfig::try_from(Arc::new(server_crypto))?,
        ));
        server_config.transport_config(Arc::clone(&transport));

        let endpoint = quinn::Endpoint::server(server_config, addr).map_err(QuicError::Bind)?;
        Ok(Self {
            endpoint,
            identity,
            provider,
            transport,
            node_id,
        })
    }

    /// The identity this endpoint presents to peers.
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// The bound local address.
    pub fn local_addr(&self) -> Result<SocketAddr, QuicError> {
        self.endpoint.local_addr().map_err(QuicError::Bind)
    }

    /// Dials `addr`, presenting `sni`, and accepts only the `expected_peer` key.
    pub async fn connect(
        &self,
        addr: SocketAddr,
        expected_peer: NodeId,
        sni: &str,
    ) -> Result<QuicLink, QuicError> {
        let client_crypto =
            auth::client_config(&self.provider, Arc::clone(&self.identity), expected_peer)?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(
            Arc::new(client_crypto),
        )?));
        client_config.transport_config(Arc::clone(&self.transport));

        let connection = self
            .endpoint
            .connect_with(client_config, addr, sni)?
            .await?;
        let remote = peer_node_id(&connection)?;
        Ok(QuicLink { connection, remote })
    }

    /// Accepts the next inbound peer, returning its authenticated identity.
    pub async fn accept(&self) -> Result<(QuicLink, NodeId), QuicError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or(QuicError::EndpointClosed)?;
        let connection = incoming.accept()?.await?;
        let remote = peer_node_id(&connection)?;
        Ok((QuicLink { connection, remote }, remote))
    }

    /// Closes the endpoint and every connection on it.
    pub fn close(&self) {
        self.endpoint.close(CLOSE_OK, b"");
    }
}

/// One authenticated QUIC connection to a peer, carrying WireGuard packets as datagrams.
#[derive(Debug, Clone)]
pub struct QuicLink {
    connection: Connection,
    remote: NodeId,
}

impl QuicLink {
    /// Sends one WireGuard datagram, unreliably. Fails if it exceeds [`Self::max_datagram_size`].
    pub fn send(&self, datagram: &[u8]) -> Result<(), QuicError> {
        self.connection
            .send_datagram(Bytes::copy_from_slice(datagram))
            .map_err(QuicError::Send)
    }

    /// Receives the next datagram from the peer.
    pub async fn recv(&self) -> Result<Bytes, QuicError> {
        self.connection
            .read_datagram()
            .await
            .map_err(QuicError::from)
    }

    /// The largest datagram the peer will currently accept, if datagrams are enabled.
    pub fn max_datagram_size(&self) -> Option<usize> {
        self.connection.max_datagram_size()
    }

    /// The peer's authenticated identity.
    #[must_use]
    pub fn remote_node(&self) -> NodeId {
        self.remote
    }

    /// The negotiated ALPN and SNI, for observing the handshake.
    pub fn handshake(&self) -> Result<LinkHandshake, QuicError> {
        let data = self
            .connection
            .handshake_data()
            .ok_or(QuicError::MalformedPeerKey)?
            .downcast::<HandshakeData>()
            .map_err(|_| QuicError::MalformedPeerKey)?;
        Ok(LinkHandshake {
            alpn: data.protocol.clone(),
            server_name: data.server_name.clone(),
        })
    }

    /// Closes the connection orderly.
    pub fn close(&self) {
        self.connection.close(CLOSE_OK, b"");
    }
}

/// What a peer negotiated during the TLS handshake.
#[derive(Debug, Clone)]
pub struct LinkHandshake {
    alpn: Option<Vec<u8>>,
    server_name: Option<String>,
}

impl LinkHandshake {
    /// The negotiated ALPN protocol.
    #[must_use]
    pub fn alpn(&self) -> Option<&[u8]> {
        self.alpn.as_deref()
    }

    /// The server name the client presented (SNI).
    #[must_use]
    pub fn server_name(&self) -> Option<&str> {
        self.server_name.as_deref()
    }
}

/// Browser-like transport parameters, datagram-only, bounded for small routers.
fn transport_config() -> Result<Arc<TransportConfig>, QuicError> {
    let mut config = TransportConfig::default();
    // WireGuard needs no streams; datagrams carry every packet.
    config.max_concurrent_bidi_streams(VarInt::from_u32(0));
    config.max_concurrent_uni_streams(VarInt::from_u32(0));
    config.datagram_receive_buffer_size(Some(DATAGRAM_RECEIVE_BUFFER));
    config.datagram_send_buffer_size(DATAGRAM_SEND_BUFFER);
    config.max_idle_timeout(Some(
        MAX_IDLE_TIMEOUT
            .try_into()
            .map_err(|_| QuicError::IdleTimeout)?,
    ));
    config.keep_alive_interval(Some(KEEP_ALIVE_INTERVAL));
    Ok(Arc::new(config))
}

/// Extracts the peer identity from the raw public key presented during the handshake.
fn peer_node_id(connection: &Connection) -> Result<NodeId, QuicError> {
    let identity = connection
        .peer_identity()
        .ok_or(QuicError::MalformedPeerKey)?;
    let chain = identity
        .downcast::<Vec<CertificateDer<'static>>>()
        .map_err(|_| QuicError::MalformedPeerKey)?;
    let spki = chain.first().ok_or(QuicError::MalformedPeerKey)?;
    der::node_id_from_spki(spki.as_ref())
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use kith_crypto::SigningKey;
    use kith_types::NodeId;

    use super::QuicEndpoint;
    use crate::ALPN;

    const SNI: &str = "www.example.com";
    const DATAGRAMS: u32 = 1000;

    fn node_id(seed: u8) -> NodeId {
        SigningKey::from_seed(&[seed; 32]).node_id()
    }

    fn loopback() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
    }

    async fn pair() -> (
        QuicEndpoint,
        super::QuicLink,
        QuicEndpoint,
        super::QuicLink,
        NodeId,
    ) {
        let server = QuicEndpoint::bind(loopback(), &[1; 32]).unwrap();
        let client = QuicEndpoint::bind(loopback(), &[2; 32]).unwrap();
        let server_addr = server.local_addr().unwrap();
        let server_node = server.node_id();

        let accept = tokio::spawn(async move {
            let (link, who) = server.accept().await.unwrap();
            (server, link, who)
        });
        let client_link = client.connect(server_addr, server_node, SNI).await.unwrap();
        let (server, server_link, accepted) = accept.await.unwrap();
        (server, server_link, client, client_link, accepted)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mutual_auth_and_a_thousand_datagrams_each_way() {
        let (server, server_link, client, client_link, accepted) = pair().await;

        assert_eq!(client_link.remote_node(), server.node_id());
        assert_eq!(accepted, client.node_id());
        assert_eq!(server_link.remote_node(), client.node_id());

        let max = client_link.max_datagram_size().expect("datagrams enabled");
        assert!(client_link.send(&vec![0_u8; max + 1]).is_err());
        client_link.send(&vec![7_u8; max]).unwrap();
        let probe = server_link.recv().await.unwrap();
        assert_eq!(probe.len(), max);
        assert!(probe.iter().all(|&b| b == 7));
        let sender = server_link.clone();
        let send = tokio::spawn(async move {
            for i in 0..DATAGRAMS {
                sender.send(&i.to_le_bytes()).unwrap();
                tokio::task::yield_now().await;
            }
        });
        for i in 0..DATAGRAMS {
            let got = client_link.recv().await.unwrap();
            assert_eq!(got.as_ref(), i.to_le_bytes());
        }
        send.await.unwrap();

        let sender = client_link.clone();
        let send = tokio::spawn(async move {
            for i in 0..DATAGRAMS {
                sender.send(&i.to_le_bytes()).unwrap();
                tokio::task::yield_now().await;
            }
        });
        for i in 0..DATAGRAMS {
            let got = server_link.recv().await.unwrap();
            assert_eq!(got.as_ref(), i.to_le_bytes());
        }
        send.await.unwrap();

        client.close();
        server.close();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrong_expected_peer_is_rejected() {
        let server = QuicEndpoint::bind(loopback(), &[1; 32]).unwrap();
        let client = QuicEndpoint::bind(loopback(), &[2; 32]).unwrap();
        let server_addr = server.local_addr().unwrap();

        let accept = tokio::spawn(async move { server.accept().await.map(|(_, who)| who) });
        // Expect a different key than the server actually holds.
        let result = client.connect(server_addr, node_id(9), SNI).await;
        assert!(result.is_err());
        drop(accept);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn alpn_and_sni_are_what_we_configured() {
        let (_server, server_link, _client, client_link, _) = pair().await;

        let client_hs = client_link.handshake().unwrap();
        assert_eq!(client_hs.alpn(), Some(ALPN));

        // The server observes both the negotiated ALPN and the SNI the client presented.
        let server_hs = server_link.handshake().unwrap();
        assert_eq!(server_hs.alpn(), Some(ALPN));
        assert_eq!(server_hs.server_name(), Some(SNI));
    }
}
