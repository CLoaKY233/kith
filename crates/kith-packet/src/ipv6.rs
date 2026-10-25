//! IPv6 fixed-header view (RFC 8200).

use core::net::Ipv6Addr;

use crate::{Error, field};

const HEADER: usize = 40;
const PAYLOAD_LEN: usize = 4;
const NEXT_HEADER: usize = 6;
const HOP_LIMIT: usize = 7;
const SRC: usize = 8;
const DST: usize = 24;

/// A validated view of an IPv6 packet's fixed header.
///
/// Invariant: version is 6 and `40 + payload_len <= buffer.len()`.
///
/// Extension headers are not walked in v1: [`next_header`](Self::next_header)
/// may name one, in which case [`payload`](Self::payload) starts with it.
/// Jumbograms (payload length zero with a Jumbo option) are not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv6Packet<T> {
    buffer: T,
}

impl<T: AsRef<[u8]>> Ipv6Packet<T> {
    /// Validates version and payload length against `buffer`.
    pub fn new_checked(buffer: T) -> Result<Self, Error> {
        let bytes = buffer.as_ref();
        let first = *bytes.first().ok_or(Error::Truncated)?;
        if first >> 4 != 6 {
            return Err(Error::BadVersion);
        }
        if bytes.len() < HEADER {
            return Err(Error::Truncated);
        }
        let payload_len = usize::from(field::u16_at(bytes, PAYLOAD_LEN));
        if payload_len > bytes.len().saturating_sub(HEADER) {
            return Err(Error::Truncated);
        }
        Ok(Self { buffer })
    }

    /// Returns the underlying buffer.
    pub fn into_inner(self) -> T {
        self.buffer
    }

    fn bytes(&self) -> &[u8] {
        self.buffer.as_ref()
    }

    /// Traffic class byte (DSCP in the upper six bits, ECN in the lower two).
    pub fn traffic_class(&self) -> u8 {
        let word = field::u16_at(self.bytes(), 0);
        let [_, low] = (word >> 4).to_be_bytes();
        low
    }

    /// Differentiated Services Code Point.
    pub fn dscp(&self) -> u8 {
        self.traffic_class() >> 2
    }

    /// Explicit Congestion Notification.
    pub fn ecn(&self) -> u8 {
        self.traffic_class() & 0x03
    }

    /// The 20-bit flow label.
    pub fn flow_label(&self) -> u32 {
        field::u32_at(self.bytes(), 0) & 0x000f_ffff
    }

    /// Payload length field: bytes after the fixed header, extension headers included.
    pub fn payload_len(&self) -> u16 {
        field::u16_at(self.bytes(), PAYLOAD_LEN)
    }

    /// The first next-header value; may be an extension header.
    pub fn next_header(&self) -> u8 {
        field::u8_at(self.bytes(), NEXT_HEADER)
    }

    /// Hop limit.
    pub fn hop_limit(&self) -> u8 {
        field::u8_at(self.bytes(), HOP_LIMIT)
    }

    /// Source address.
    pub fn src(&self) -> Ipv6Addr {
        Ipv6Addr::from(field::bytes::<16>(self.bytes(), SRC))
    }

    /// Destination address.
    pub fn dst(&self) -> Ipv6Addr {
        Ipv6Addr::from(field::bytes::<16>(self.bytes(), DST))
    }

    /// The bytes after the fixed header, up to `payload_len`.
    pub fn payload(&self) -> &[u8] {
        let end = HEADER.saturating_add(usize::from(self.payload_len()));
        field::slice(self.bytes(), HEADER, end)
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]>> Ipv6Packet<T> {
    /// The payload, mutably.
    pub fn payload_mut(&mut self) -> &mut [u8] {
        let end = HEADER.saturating_add(usize::from(self.payload_len()));
        self.buffer.as_mut().get_mut(HEADER..end).unwrap_or(&mut [])
    }

    /// Sets the hop limit (IPv6 has no header checksum).
    pub fn set_hop_limit(&mut self, hop_limit: u8) {
        field::set_bytes(self.buffer.as_mut(), HOP_LIMIT, [hop_limit]);
    }
}
