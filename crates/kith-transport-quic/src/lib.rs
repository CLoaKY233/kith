//! QUIC-on-443 link driver.
//!
//! Plane: connectivity. Carries already-encrypted WireGuard datagrams inside QUIC DATAGRAM
//! frames (RFC 9221) on UDP/443 with ALPN `h3`, so a passive observer sees HTTP/3. Peers
//! authenticate with their Ed25519 kith identity as a TLS 1.3 raw public key (RFC 7250): no CA,
//! no certificates. See `docs/architecture.md`.
//!
//! Indistinguishability: the destination port, ALPN, SNI and QUIC long-header shape match
//! HTTP/3. Transport parameters (including the zero stream limits) travel inside the encrypted
//! handshake, so a passive observer cannot read them; an active probe that completes a TLS
//! handshake still learns nothing, since the raw-public-key `Certificate` message differs from a
//! web server's X.509 chain. This rung defeats passive fingerprinting, not a full active probe.

#![forbid(unsafe_code)]

use std::time::Duration;

mod auth;
mod der;
mod endpoint;

pub use endpoint::{LinkHandshake, QuicEndpoint, QuicLink};

/// ALPN identifier advertised and required on every connection.
pub const ALPN: &[u8] = b"h3";

/// Idle timeout before an unused connection is dropped.
pub const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Keep-alive interval; kept below [`MAX_IDLE_TIMEOUT`] so a quiet link stays up.
pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Datagram receive-buffer cap in bytes, sized small for routers.
pub const DATAGRAM_RECEIVE_BUFFER: usize = 256 * 1024;

/// Datagram send-buffer cap in bytes, sized small for routers.
pub const DATAGRAM_SEND_BUFFER: usize = 256 * 1024;

/// Something went wrong binding, connecting, authenticating, or moving datagrams.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum QuicError {
    /// Binding the UDP socket or reading its address failed.
    #[error("bind: {0}")]
    Bind(#[source] std::io::Error),
    /// The connection could not be started.
    #[error("connect: {0}")]
    Connect(#[from] quinn::ConnectError),
    /// An established connection failed or closed.
    #[error("connection: {0}")]
    Connection(#[from] quinn::ConnectionError),
    /// The TLS configuration is invalid.
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    /// The configured crypto provider lacks the mandatory initial cipher suite.
    #[error("no initial cipher suite")]
    Cipher(#[from] quinn::crypto::rustls::NoInitialCipherSuite),
    /// Sending a datagram failed (too large, or datagrams unsupported by the peer).
    #[error("send datagram: {0}")]
    Send(#[source] quinn::SendDatagramError),
    /// A key supplied by the caller or the peer is not a valid Ed25519 identity.
    #[error("key: {0}")]
    Key(#[from] kith_crypto::SignatureError),
    /// The peer presented no raw public key, or one of the wrong shape.
    #[error("malformed peer key")]
    MalformedPeerKey,
    /// The endpoint has been closed.
    #[error("endpoint closed")]
    EndpointClosed,
    /// The idle-timeout constant does not fit a QUIC transport parameter.
    #[error("idle timeout out of range")]
    IdleTimeout,
}
