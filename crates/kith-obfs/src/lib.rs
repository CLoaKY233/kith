//! Packet obfuscation transforms: junk packets, padding and header remapping.
//!
//! Plane: connectivity. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! # Why
//!
//! Plain WireGuard is easy for deep packet inspection to fingerprint. Every
//! message starts with a fixed little-endian type word (`1`..`4`), and the
//! handshake messages have fixed sizes (148 and 92 bytes). This crate is the
//! first rung of kith's stealth ladder. It is an AmneziaWG 1.x compatible
//! transform that changes only the *framing* of messages that are already
//! sealed. The WireGuard cryptography is untouched, so a kith node can talk to
//! stock AmneziaWG servers.
//!
//! The transform does three things:
//!
//! - **Header remap.** The 4-byte type word of each message is replaced by a
//!   per-network magic value (`H1`..`H4`).
//! - **Handshake padding.** `S1` (initiation) and `S2` (response) random bytes
//!   are prepended, so handshake sizes are no longer fixed.
//! - **Junk packets.** Before each handshake initiation, `Jc` datagrams of
//!   random bytes with a random length in `[Jmin, Jmax]` are sent. The receiver
//!   drops them.
//!
//! # Hot path
//!
//! Transport data is the hot path. There, the transform is a 4-byte header
//! rewrite in place ([`ObfsParams::encode_in_place`] and
//! [`ObfsParams::decode`]). Nothing in this crate allocates.
//!
//! # Randomness
//!
//! The crate stays sans-IO: callers supply randomness through
//! [`RandomSource`]. Padding and junk need no cryptographic strength, but
//! [`ObfsParams::random`] should be fed by a CSPRNG.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

// Kept per the core-crate convention; nothing here allocates yet (AWG 2.0
// signature packets may).
#[cfg_attr(
    not(test),
    expect(unused_extern_crates, reason = "core-crate convention: no_std + alloc")
)]
extern crate alloc;

mod params;
mod random;
mod transform;

pub use params::{
    MAX_JUNK_COUNT, ObfsParams, ParamsError, RawObfsParams, WG_COOKIE_LEN, WG_INITIATION_LEN,
    WG_RESPONSE_LEN, WG_TRANSPORT_MIN_LEN,
};
pub use random::RandomSource;
pub use transform::{EncodeError, Inbound, MessageType};
#[cfg(test)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::wildcard_enum_match_arm,
    reason = "tests use fixed small values and fail loudly"
)]
mod tests;
