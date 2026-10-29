//! Zero-copy IPv4, IPv6, TCP, UDP and ICMP header views and checksums.
//!
//! Sans-IO, `no_std`, no `alloc`. Each view wraps a caller-owned `T: AsRef<[u8]>`
//! (setters need `AsMut`). `new_checked` validates every length a getter relies
//! on; getters avoid indexing so a validation bug yields a wrong value, not a panic.
//!
//! Checksums: RFC 1071 (full) and RFC 1624 (incremental), see `checksum`.
//!
//! Scope (v1): IPv6 extension headers are not walked, IP options are skipped,
//! IPv6 jumbograms are rejected.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

#[cfg(test)]
extern crate std;

mod checksum;
mod error;
mod field;
mod flow;
mod icmp;
mod ip;
mod ipv4;
mod ipv6;
mod mss;
mod tcp;

#[cfg(test)]
mod tests;

pub use error::Error;
pub use flow::FlowKey;
pub use icmp::IcmpPacket;
pub use ip::{IpPacket, PseudoHeader};
pub use ipv4::Ipv4Packet;
pub use ipv6::Ipv6Packet;
pub use mss::{clamp_mss, needs_mss_clamp};
pub use tcp::{TcpFlags, TcpPacket};

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
