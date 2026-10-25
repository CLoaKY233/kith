//! Zero-copy IPv4, IPv6, TCP, UDP and ICMP header views and checksums.
//!
//! Plane: data. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! # Why this crate exists
//!
//! Every packet that crosses the tunnel is inspected by the packet filter and
//! the router. That inspection must not copy, allocate or panic, so this crate
//! offers thin *views* over a caller-owned buffer in the style of smoltcp's
//! `Packet<T>` wrappers:
//!
//! - Each view is generic over `T: AsRef<[u8]>`. Setters additionally need
//!   `T: AsMut<[u8]>`.
//! - `new_checked` validates every length a getter relies on, once. Getters
//!   then read fixed offsets and never fail. They are still written without
//!   indexing, so a bug degrades to a wrong value rather than a panic.
//! - The crate is `no_std` with no `alloc`: it runs unchanged on routers and
//!   microcontrollers.
//!
//! [`FlowKey::extract`] is the one-call entry point for the hot path. The
//! checksum helpers in [`checksum`] implement RFC 1071 (full) and RFC 1624
//! (incremental) arithmetic.
//!
//! # Scope (v1)
//!
//! IPv6 extension headers are not walked: [`Ipv6Packet::next_header`] is the
//! first next-header value, and [`FlowKey`] carries no ports when it is not
//! TCP or UDP. IP options are skipped, not parsed. IPv6 jumbograms are rejected.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

#[cfg(test)]
extern crate std;

pub mod checksum;
mod error;
mod field;
mod flow;
mod icmp;
mod ip;
mod ipv4;
mod ipv6;
mod tcp;
mod udp;

#[cfg(test)]
mod tests;

pub use error::Error;
pub use flow::FlowKey;
pub use icmp::IcmpPacket;
pub use ip::{IpPacket, PseudoHeader};
pub use ipv4::Ipv4Packet;
pub use ipv6::Ipv6Packet;
pub use tcp::{TcpFlags, TcpPacket};
pub use udp::UdpPacket;

/// IANA protocol numbers used by this crate (the IPv4 `protocol` field and
/// the IPv6 `next_header` field share one registry).
pub mod ip_protocol {
    /// Internet Control Message Protocol for IPv4.
    pub const ICMP: u8 = 1;
    /// Transmission Control Protocol.
    pub const TCP: u8 = 6;
    /// User Datagram Protocol.
    pub const UDP: u8 = 17;
    /// Internet Control Message Protocol for IPv6.
    pub const ICMPV6: u8 = 58;
}
