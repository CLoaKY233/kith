//! A sans-IO WireGuard protocol core, interoperable with stock WireGuard.
//!
//! Plane: data. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! Time ([`Now`]) and randomness ([`RandomSource`]) are inputs; datagrams go into
//! caller buffers, and the transport path never allocates. [`Peer`] is one peer's state
//! machine; [`Device`] routes datagrams to peers by static key or receiver index.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod cookie;
mod device;
mod error;
mod messages;
mod noise;
mod peer;
mod random;
mod replay;
mod session;
mod time;

pub use cookie::CookieChecker;
pub use device::{Device, DeviceOutput, PeerId};
pub use error::WgError;
pub use messages::{MAX_OUTPUT_OVERHEAD, MIN_OUTPUT_LEN, required_output_len};
pub use noise::StaticIdentity;
pub use peer::{Decapsulated, Encapsulated, MAX_QUEUED_PACKETS, Peer, PeerConfig, Transmit};
pub use random::RandomSource;
pub use time::{Now, Tai64N};
