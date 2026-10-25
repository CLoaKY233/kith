//! UDP header view (RFC 768).

use crate::{Error, PseudoHeader, field, ip_protocol};

const HEADER: usize = 8;
const LENGTH: usize = 4;
const CHECKSUM: usize = 6;

/// A validated view of a UDP datagram (the IP payload).
///
/// Invariant: `8 <= length <= buffer.len()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpPacket<T> {
    buffer: T,
}

impl<T: AsRef<[u8]>> UdpPacket<T> {
    /// Validates the header and the length field against `buffer`.
    pub fn new_checked(buffer: T) -> Result<Self, Error> {
        let bytes = buffer.as_ref();
        if bytes.len() < HEADER {
            return Err(Error::Truncated);
        }
        let length = usize::from(field::u16_at(bytes, LENGTH));
        if length < HEADER {
            return Err(Error::BadTotalLength);
        }
        if length > bytes.len() {
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

    /// Source port.
    pub fn src_port(&self) -> u16 {
        field::u16_at(self.bytes(), 0)
    }

    /// Destination port.
    pub fn dst_port(&self) -> u16 {
        field::u16_at(self.bytes(), 2)
    }

    /// Length field: header plus payload, in bytes.
    pub fn length(&self) -> u16 {
        field::u16_at(self.bytes(), LENGTH)
    }

    /// Stored checksum; zero means "none" over IPv4.
    pub fn checksum(&self) -> u16 {
        field::u16_at(self.bytes(), CHECKSUM)
    }

    /// The datagram as covered by `length`.
    fn datagram(&self) -> &[u8] {
        field::slice(self.bytes(), 0, usize::from(self.length()))
    }

    /// The payload after the 8-byte header.
    pub fn payload(&self) -> &[u8] {
        field::slice(self.bytes(), HEADER, usize::from(self.length()))
    }

    /// Whether the checksum is valid. A zero checksum over IPv4 means the
    /// sender did not compute one and is accepted; over IPv6 it is invalid.
    pub fn verify_checksum(&self, pseudo_header: &PseudoHeader) -> bool {
        if self.checksum() == 0 {
            return pseudo_header.is_v4();
        }
        pseudo_header.checksum(ip_protocol::UDP, self.datagram()) == 0
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]>> UdpPacket<T> {
    /// Computes and stores the checksum. A computed zero is sent as `0xffff`.
    pub fn fill_checksum(&mut self, pseudo_header: &PseudoHeader) {
        field::set_u16(self.buffer.as_mut(), CHECKSUM, 0);
        let value = match pseudo_header.checksum(ip_protocol::UDP, self.datagram()) {
            0 => 0xffff,
            other => other,
        };
        field::set_u16(self.buffer.as_mut(), CHECKSUM, value);
    }
}
