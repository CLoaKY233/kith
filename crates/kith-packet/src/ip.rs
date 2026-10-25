//! Version-dispatching IP view and the transport pseudo-header.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::checksum::Checksum;
use crate::{Error, Ipv4Packet, Ipv6Packet};

/// An IPv4 or IPv6 packet, chosen by the version nibble.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpPacket<T> {
    /// An IPv4 packet.
    V4(Ipv4Packet<T>),
    /// An IPv6 packet.
    V6(Ipv6Packet<T>),
}

impl<T: AsRef<[u8]>> IpPacket<T> {
    /// Validates `buffer` as IPv4 or IPv6 according to its first nibble.
    pub fn new_checked(buffer: T) -> Result<Self, Error> {
        match buffer.as_ref().first().map(|byte| byte >> 4) {
            None => Err(Error::Truncated),
            Some(4) => Ipv4Packet::new_checked(buffer).map(Self::V4),
            Some(6) => Ipv6Packet::new_checked(buffer).map(Self::V6),
            Some(_) => Err(Error::BadVersion),
        }
    }

    /// Source address.
    pub fn src(&self) -> IpAddr {
        match self {
            Self::V4(packet) => IpAddr::V4(packet.src()),
            Self::V6(packet) => IpAddr::V6(packet.src()),
        }
    }

    /// Destination address.
    pub fn dst(&self) -> IpAddr {
        match self {
            Self::V4(packet) => IpAddr::V4(packet.dst()),
            Self::V6(packet) => IpAddr::V6(packet.dst()),
        }
    }

    /// IPv4 protocol, or the first IPv6 next header (extension headers are not walked).
    pub fn protocol(&self) -> u8 {
        match self {
            Self::V4(packet) => packet.protocol(),
            Self::V6(packet) => packet.next_header(),
        }
    }

    /// The upper-layer bytes.
    pub fn payload(&self) -> &[u8] {
        match self {
            Self::V4(packet) => packet.payload(),
            Self::V6(packet) => packet.payload(),
        }
    }

    /// The pseudo-header for transport checksums over this packet.
    pub fn pseudo_header(&self) -> PseudoHeader {
        match self {
            Self::V4(packet) => PseudoHeader::V4 {
                src: packet.src(),
                dst: packet.dst(),
            },
            Self::V6(packet) => PseudoHeader::V6 {
                src: packet.src(),
                dst: packet.dst(),
            },
        }
    }
}

/// The addresses that TCP, UDP and `ICMPv6` checksums cover.
///
/// An enum, so that mixing an IPv4 source with an IPv6 destination is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoHeader {
    /// IPv4 pseudo-header (RFC 768, RFC 9293).
    V4 {
        /// Source address.
        src: Ipv4Addr,
        /// Destination address.
        dst: Ipv4Addr,
    },
    /// IPv6 pseudo-header (RFC 8200 section 8.1).
    V6 {
        /// Source address.
        src: Ipv6Addr,
        /// Destination address.
        dst: Ipv6Addr,
    },
}

impl PseudoHeader {
    /// Whether this is an IPv4 pseudo-header.
    pub fn is_v4(&self) -> bool {
        matches!(self, Self::V4 { .. })
    }

    /// The stored checksum for `segment` (whose checksum field must be zero)
    /// carried with `protocol`; equivalently, zero iff an in-place checksum verifies.
    pub(crate) fn checksum(&self, protocol: u8, segment: &[u8]) -> u16 {
        let mut sum = Checksum::new();
        match self {
            Self::V4 { src, dst } => {
                sum.add_bytes(&src.octets());
                sum.add_bytes(&dst.octets());
            }
            Self::V6 { src, dst } => {
                sum.add_bytes(&src.octets());
                sum.add_bytes(&dst.octets());
            }
        }
        sum.add_value(u64::from(protocol));
        // usize -> u64 is lossless on every supported target.
        sum.add_value(u64::try_from(segment.len()).unwrap_or(u64::MAX));
        sum.add_bytes(segment);
        sum.finish()
    }
}
