//! Relay frame codec and client/server state machines.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

pub mod auth;
mod client;
mod frame;
mod server;

#[cfg(test)]
mod tests;

pub use client::{ClientError, Event, RelayClient, SendError};
pub use frame::{
    DecodeError, Decoder, Frame, FrameTooLarge, HealthProblem, MAX_PAYLOAD_LEN, NONCE_LEN,
    RejectReason,
};
pub use server::{
    Action, Authorizer, CloseReason, ConnId, ConnectionStats, DuplicateConnection, RelayServer,
    ServerStats,
};
