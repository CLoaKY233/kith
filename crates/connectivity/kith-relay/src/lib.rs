//! Relay server and client drivers.
//!
//! Plane: connectivity. Driver: owns I/O and wraps the sans-IO `kith-relay-proto` cores. See
//! `docs/architecture.md`. Connection logic is generic over `AsyncRead + AsyncWrite`; TCP and
//! TLS are the provided carriers.

#![forbid(unsafe_code)]

mod client;
mod server;
mod tls;

pub use client::{Client, ClientOptions, Incoming, SendError, Status, Verification};
pub use kith_relay_proto::{Authorizer, RejectReason};
pub use server::{Server, ServerOptions, TlsIdentity};

/// A relay driver failed to start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A socket operation failed.
    #[error("relay I/O: {0}")]
    Io(#[from] std::io::Error),
    /// The TLS configuration was invalid.
    #[error("relay TLS: {0}")]
    Tls(#[from] rustls::Error),
    /// The server name is not a valid DNS name or IP address.
    #[error("invalid TLS server name")]
    InvalidServerName,
}

/// Bytes read from a stream per call.
const READ_CHUNK: usize = 16 * 1024;
