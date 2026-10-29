//! Protocol between the `kith` command line and the daemon.
//!
//! Plane: management. Library. See `docs/architecture.md`.
//!
//! Every message is a `kith-wire` envelope ([`kith_wire::registry::IPC_MESSAGE`]) inside a
//! length-prefixed frame. A connection starts with [`Request::Hello`]; the server answers
//! every request with exactly one [`Response`], except that after [`Request::Watch`] it
//! only sends [`Event`]s.

#![forbid(unsafe_code)]

mod client;
mod codec;
mod error;
mod message;
mod server;

pub use client::IpcClient;
pub use error::IpcError;
pub use message::{
    DnsRecord, ErrorCode, Event, NetworkSummary, PathKind, PeerInfo, Request, Response, Status,
    Warning,
};
pub use server::{Connection, IpcServer, PeerCred};

/// The protocol version both sides must agree on in [`Request::Hello`].
pub const PROTOCOL_VERSION: u32 = 7;

/// The largest frame either side accepts.
pub const MAX_FRAME_LEN: usize = 256 * 1024;

/// How long the client waits for a response.
pub const REQUEST_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(30);

#[cfg(test)]
mod tests;
