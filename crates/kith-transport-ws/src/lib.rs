//! TLS and WebSocket link driver, CDN-compatible.
//!
//! Plane: connectivity. Driver: owns I/O and wraps sans-IO cores. See `docs/architecture.md`.
//!
//! Each binary WebSocket message carries exactly one datagram. The RFC 6455 codec is
//! hand-written and sans-IO (`handshake`, `frame`); this crate does no authentication,
//! the relay protocol on top does.

#![forbid(unsafe_code)]

mod client;
mod error;
mod frame;
mod handshake;
mod link;
mod server;
mod sha1;
mod stream;

pub use client::{ClientOptions, connect};
pub use error::Error;
pub use link::{KEEPALIVE_INTERVAL, MAX_DATAGRAM, WsLink, WsSender};
pub use server::{Incoming, TlsIdentity, WsServer};
