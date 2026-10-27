//! Peer discovery messages, endpoint candidates and path selection.
//!
//! Sans-IO: no I/O, no clock, no runtime. kith's equivalent of Tailscale's disco and magicsock:
//!
//! - [`message`]: [`Ping`], [`Pong`] and [`CallMeMaybe`], with a byte-stable encoding.
//! - [`seal`](mod@seal): authenticated encryption between two nodes' disco keys, so a pong cannot be
//!   forged by anyone on the path, plus [`is_disco`] for demultiplexing the UDP socket.
//! - [`paths`]: [`PeerPaths`], the per-peer state machine that probes candidate endpoints,
//!   measures latency and picks direct UDP over the relay.
//!
//! Cryptographic contexts owned by this crate:
//!
//! - `"kith 2026-10 disco seal v1"` ([`seal::SEAL_CONTEXT`]): per-packet sealing keys.
//! - `"kith 2026-10 disco tx id v1"` ([`paths::TX_ID_CONTEXT`]): transaction IDs.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

pub mod message;
pub mod paths;
pub mod seal;

pub use message::{CallMeMaybe, Message, ParseError, Ping, Pong, TxId};
pub use paths::{Event, PathReason, PathTarget, PeerPaths, Transmit};
pub use seal::{OpenError, RandomSource, SealError, is_disco, open, seal};
