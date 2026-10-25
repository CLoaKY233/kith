//! Relay server and client drivers over any `AsyncRead + AsyncWrite` stream, with TCP and TLS.

#![forbid(unsafe_code)]

mod client;
mod server;
mod tls;

use std::io;

use kith_relay_proto::{ClientError, CloseReason, DecodeError, FrameTooLarge};
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub use client::{Client, ClientOptions, Incoming, SendError, Status, Verification};
pub use kith_relay_proto::{Authorizer, RejectReason};
pub use server::{Server, ServerOptions, TlsIdentity};

/// A relay driver failed to start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A socket operation failed.
    #[error("relay I/O: {0}")]
    Io(#[from] io::Error),
    /// The TLS configuration was invalid.
    #[error("relay TLS: {0}")]
    Tls(#[from] rustls::Error),
    /// The server name is not a valid DNS name or IP address.
    #[error("invalid TLS server name")]
    InvalidServerName,
}

const READ_CHUNK: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
enum Ended {
    #[error("closed by the peer")]
    PeerClosed,
    #[error("idle timeout")]
    IdleTimeout,
    #[error("I/O: {0}")]
    Io(io::Error),
    #[error("{0}")]
    Malformed(DecodeError),
    #[error("{0}")]
    Unencodable(FrameTooLarge),
    #[error("{0}")]
    Misbehaved(ClientError),
    #[error("rejected with code {}", .0.code())]
    Rejected(RejectReason),
    #[error("closed by the relay: {0:?}")]
    Closed(CloseReason),
    #[error("stopped")]
    Stopped,
}

async fn write_flushed<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> Result<(), Ended> {
    writer.write_all(bytes).await.map_err(Ended::Io)?;
    writer.flush().await.map_err(Ended::Io)
}
