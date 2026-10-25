//! Minimal STUN (RFC 8489) binding request and response codec, sans-IO and allocation-free.
//!
//! Implements only what NAT traversal needs, the same scope as Tailscale's `net/stun`:
//! a client ([`StunProbe`]), a server side, and [`is_stun`] to demultiplex STUN
//! from WireGuard on one socket.
//!
//! MESSAGE-INTEGRITY is never verified: a forged reply can only mislead us about
//! our own address, and disco verifies every path end to end. FINGERPRINT is
//! always sent and verified when present.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

mod crc32;
mod message;
mod probe;

pub use message::{
    MAX_RESPONSE_LEN, REQUEST_LEN, StunError, TransactionId, encode_binding_request,
    encode_binding_response, is_stun, parse_binding_request, parse_binding_response,
};
pub use probe::{GIVE_UP_AFTER, RETRANSMIT_SCHEDULE, StunProbe};

#[cfg(test)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::unchecked_time_subtraction,
    reason = "tests compute small fixed offsets and indexes; overflow would be a test bug"
)]
mod tests;
