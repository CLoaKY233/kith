//! Relay frame codec and client/server state machines.
//!
//! Sans-IO: no I/O, no clock, no runtime. A kith relay serves only members of its network,
//! so every connection proves key possession with a signed challenge before it may send.
//! Payloads are opaque, already-encrypted WireGuard packets. Drivers live in `kith-relay`.

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
