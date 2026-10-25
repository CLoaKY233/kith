//! Relay frame codec and client/server state machines.
//!
//! Plane: connectivity. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! When two devices cannot reach each other directly, they send their already-encrypted
//! WireGuard packets through a relay: any kith node, often a user's own server. Unlike a
//! public relay, a kith relay serves only members of its network, so every connection
//! proves key possession with a signed challenge before it may send anything.
//!
//! - [`frame`]: the wire format and an incremental [`Decoder`].
//! - [`auth`]: the challenge signature, bound to the nonce and the relay's identity.
//! - [`server`]: [`RelayServer`], which authenticates, forwards and bounds memory.
//! - [`client`]: [`RelayClient`], which answers the challenge and exchanges packets.
//!
//! The relay never sees plaintext: payloads are opaque. Drivers that own the streams live in
//! `kith-relay`.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

pub mod auth;
pub mod client;
pub mod frame;
pub mod server;

#[cfg(test)]
mod tests;

pub use client::{ClientError, Event, RelayClient, SendError};
pub use frame::{Decoder, Frame, HealthProblem, RejectReason};
pub use server::{Action, Authorizer, CloseReason, ConnId, RelayServer};
