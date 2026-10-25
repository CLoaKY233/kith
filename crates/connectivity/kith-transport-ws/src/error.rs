use std::io;

/// Errors from connecting, accepting or using a WebSocket link.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Socket I/O failed.
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    /// TLS setup or handshake failed.
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    /// The URL is not `wss://host[:port]/path`.
    #[error("invalid url: {0}")]
    InvalidUrl(&'static str),
    /// A host, path or header value contains characters HTTP does not allow.
    #[error("invalid http field: {0}")]
    InvalidField(&'static str),
    /// The HTTP upgrade exchange was malformed.
    #[error("bad handshake: {0}")]
    Handshake(&'static str),
    /// The server answered with a status other than 101.
    #[error("server refused the upgrade with status {0}")]
    Rejected(u16),
    /// The peer violated RFC 6455.
    #[error("websocket protocol violation: {0}")]
    Protocol(&'static str),
    /// A datagram exceeds [`crate::MAX_DATAGRAM`].
    #[error("datagram too large")]
    TooLarge,
    /// The peer stopped answering.
    #[error("timed out")]
    Timeout,
    /// The link was closed.
    #[error("link closed")]
    Closed,
    /// The OS random source failed.
    #[error("random source unavailable")]
    Random,
}

impl From<kith_crypto::RandomError> for Error {
    fn from(_: kith_crypto::RandomError) -> Self {
        Self::Random
    }
}
