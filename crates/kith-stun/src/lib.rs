//! Minimal STUN binding request and response codec.
//!
//! Plane: connectivity. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! NAT traversal needs each device to learn its public `ip:port` as seen from
//! the internet. STUN (RFC 8489, formerly RFC 5389) answers exactly that with a
//! single round trip: the client sends a Binding request and the server echoes
//! the source address it observed. Kith needs nothing else from STUN, so this
//! crate implements only that subset, the same scope as Tailscale's `net/stun`:
//!
//! - **Client:** [`encode_binding_request`], [`parse_binding_response`] and the
//!   retransmitting prober [`StunProbe`].
//! - **Server:** [`parse_binding_request`] and [`encode_binding_response`], so
//!   any kith node can answer STUN for its peers.
//! - **Demultiplexing:** [`is_stun`] cheaply separates STUN from WireGuard
//!   traffic arriving on the same UDP socket.
//!
//! There is no authentication (MESSAGE-INTEGRITY is skipped, never verified):
//! a forged reply can only mislead us about our own address, and disco
//! verifies every path end to end before it is used. FINGERPRINT is always
//! sent and is verified whenever it is present.
//!
//! Everything is allocation-free: encoders write into caller-provided
//! fixed-size buffers and parsers borrow the packet.

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
