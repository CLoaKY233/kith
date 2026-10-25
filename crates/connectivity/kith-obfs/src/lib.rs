//! AmneziaWG 1.x compatible packet obfuscation: junk packets, padding and header remapping.
//!
//! The first rung of kith's stealth ladder. It changes only the framing of
//! already-sealed WireGuard messages, so kith interoperates with stock AmneziaWG:
//!
//! - **Header remap:** each 4-byte type word becomes a per-network `H1`..`H4`.
//! - **Handshake padding:** `S1`/`S2` random bytes prefix initiations/responses.
//! - **Junk packets:** `Jc` random datagrams of length `[Jmin, Jmax]` precede each initiation.
//!
//! Transport data is a 4-byte in-place rewrite. Nothing allocates. Randomness
//! comes from the caller's [`RandomSource`]; [`ObfsParams::random`] needs a CSPRNG.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

#[cfg_attr(
    not(test),
    expect(unused_extern_crates, reason = "core-crate convention: no_std + alloc")
)]
extern crate alloc;

mod params;
mod random;
mod transform;

pub use params::{ObfsParams, ParamsError, RawObfsParams};
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
