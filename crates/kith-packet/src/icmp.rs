//! `ICMPv4` (RFC 792) and `ICMPv6` (RFC 4443) header view.

use core::net::Ipv6Addr;

use crate::{Error, PseudoHeader, checksum, field, ip_protocol};

const HEADER: usize = 4;
const CHECKSUM: usize = 2;

/// A validated view of an `ICMPv4` or `ICMPv6` message (the IP payload).
///
/// Both versions share the type, code and checksum layout. They differ in
/// checksum coverage: `ICMPv6` includes the IPv6 pseudo-header, `ICMPv4` does not.
///
/// Invariant: `buffer.len() >= 4`. The message is the whole buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcmpPacket<T> {
    buffer: T,
}

impl<T: AsRef<[u8]>> IcmpPacket<T> {
    /// Validates that `buffer` holds the 4-byte common header.
    pub fn new_checked(buffer: T) -> Result<Self, Error> {
        if buffer.as_ref().len() < HEADER {
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

    /// Message type.
    pub fn message_type(&self) -> u8 {
        field::u8_at(self.bytes(), 0)
    }

    /// Message code.
    pub fn code(&self) -> u8 {
        field::u8_at(self.bytes(), 1)
    }

    /// Stored checksum.
    pub fn checksum(&self) -> u16 {
        field::u16_at(self.bytes(), CHECKSUM)
    }

    /// Everything after the 4-byte common header (for echo: identifier,
    /// sequence number, then data).
    pub fn payload(&self) -> &[u8] {
        self.bytes().get(HEADER..).unwrap_or(&[])
    }

    /// Whether an `ICMPv4` checksum is valid.
    pub fn verify_checksum_v4(&self) -> bool {
        checksum::checksum(self.bytes()) == 0
    }

    /// Whether an `ICMPv6` checksum is valid for the given addresses.
    pub fn verify_checksum_v6(&self, src: Ipv6Addr, dst: Ipv6Addr) -> bool {
        PseudoHeader::V6 { src, dst }.checksum(ip_protocol::ICMPV6, self.bytes()) == 0
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]>> IcmpPacket<T> {
    /// Computes and stores an `ICMPv4` checksum.
    pub fn fill_checksum_v4(&mut self) {
        field::set_u16(self.buffer.as_mut(), CHECKSUM, 0);
        let value = checksum::checksum(self.bytes());
        field::set_u16(self.buffer.as_mut(), CHECKSUM, value);
    }

    /// Computes and stores an `ICMPv6` checksum for the given addresses.
    pub fn fill_checksum_v6(&mut self, src: Ipv6Addr, dst: Ipv6Addr) {
        field::set_u16(self.buffer.as_mut(), CHECKSUM, 0);
        let value = PseudoHeader::V6 { src, dst }.checksum(ip_protocol::ICMPV6, self.bytes());
        field::set_u16(self.buffer.as_mut(), CHECKSUM, value);
    }
}
