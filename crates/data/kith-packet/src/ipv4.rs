//! IPv4 header view (RFC 791).

use core::net::Ipv4Addr;

use crate::checksum::{self, update_bytes, update_u16};
use crate::{Error, field};

const MIN_HEADER: usize = 20;
const TOTAL_LEN: usize = 2;
const FLAGS_FRAGMENT: usize = 6;
const TTL: usize = 8;
const PROTOCOL: usize = 9;
const CHECKSUM: usize = 10;
const SRC: usize = 12;
const DST: usize = 16;

/// A validated view of an IPv4 packet.
///
/// Invariant: version is 4, `20 <= header_len <= total_len <= buffer.len()`.
/// Bytes past `total_len` (link padding) are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Packet<T> {
    buffer: T,
}

impl<T: AsRef<[u8]>> Ipv4Packet<T> {
    /// Validates version, IHL, total length and header length against `buffer`.
    pub fn new_checked(buffer: T) -> Result<Self, Error> {
        let bytes = buffer.as_ref();
        let first = *bytes.first().ok_or(Error::Truncated)?;
        if first >> 4 != 4 {
            return Err(Error::BadVersion);
        }
        if bytes.len() < MIN_HEADER {
            return Err(Error::Truncated);
        }
        let header_len = usize::from(first & 0x0f).saturating_mul(4);
        if header_len < MIN_HEADER {
            return Err(Error::BadHeaderLength);
        }
        let total_len = usize::from(field::u16_at(bytes, TOTAL_LEN));
        if total_len > bytes.len() {
            return Err(Error::Truncated);
        }
        if header_len > total_len {
            return Err(Error::BadTotalLength);
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

    /// Header length in bytes (IHL × 4), including options.
    pub fn header_len(&self) -> usize {
        usize::from(field::u8_at(self.bytes(), 0) & 0x0f).saturating_mul(4)
    }

    /// Differentiated Services Code Point (upper six bits of the TOS byte).
    pub fn dscp(&self) -> u8 {
        field::u8_at(self.bytes(), 1) >> 2
    }

    /// Explicit Congestion Notification (lower two bits of the TOS byte).
    pub fn ecn(&self) -> u8 {
        field::u8_at(self.bytes(), 1) & 0x03
    }

    /// Total length field: header plus payload, in bytes.
    pub fn total_len(&self) -> u16 {
        field::u16_at(self.bytes(), TOTAL_LEN)
    }

    /// Identification field.
    pub fn identification(&self) -> u16 {
        field::u16_at(self.bytes(), 4)
    }

    /// The three flag bits (reserved, DF, MF) as the low bits of a byte.
    pub fn flags(&self) -> u8 {
        field::u8_at(self.bytes(), FLAGS_FRAGMENT) >> 5
    }

    /// Don't Fragment flag.
    pub fn dont_fragment(&self) -> bool {
        self.flags() & 0b010 != 0
    }

    /// More Fragments flag.
    pub fn more_fragments(&self) -> bool {
        self.flags() & 0b001 != 0
    }

    /// Fragment offset in 8-byte units. Non-zero means this is not the first fragment.
    pub fn fragment_offset(&self) -> u16 {
        field::u16_at(self.bytes(), FLAGS_FRAGMENT) & 0x1fff
    }

    /// Time to live.
    pub fn ttl(&self) -> u8 {
        field::u8_at(self.bytes(), TTL)
    }

    /// Upper-layer protocol number (see [`ip_protocol`](crate::ip_protocol)).
    pub fn protocol(&self) -> u8 {
        field::u8_at(self.bytes(), PROTOCOL)
    }

    /// Stored header checksum.
    pub fn header_checksum(&self) -> u16 {
        field::u16_at(self.bytes(), CHECKSUM)
    }

    /// Source address.
    pub fn src(&self) -> Ipv4Addr {
        Ipv4Addr::from(field::bytes::<4>(self.bytes(), SRC))
    }

    /// Destination address.
    pub fn dst(&self) -> Ipv4Addr {
        Ipv4Addr::from(field::bytes::<4>(self.bytes(), DST))
    }

    /// The header bytes, including options.
    pub fn header(&self) -> &[u8] {
        field::slice(self.bytes(), 0, self.header_len())
    }

    /// The payload: bytes from the end of the header to `total_len`.
    pub fn payload(&self) -> &[u8] {
        field::slice(
            self.bytes(),
            self.header_len(),
            usize::from(self.total_len()),
        )
    }

    /// Whether the header checksum is valid.
    pub fn verify_checksum(&self) -> bool {
        checksum::checksum(self.header()) == 0
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]>> Ipv4Packet<T> {
    fn bytes_mut(&mut self) -> &mut [u8] {
        self.buffer.as_mut()
    }

    /// The payload, mutably.
    pub fn payload_mut(&mut self) -> &mut [u8] {
        let (start, end) = (self.header_len(), usize::from(self.total_len()));
        self.bytes_mut().get_mut(start..end).unwrap_or(&mut [])
    }

    /// Recomputes and stores the header checksum.
    pub fn fill_checksum(&mut self) {
        let header_len = self.header_len();
        field::set_u16(self.bytes_mut(), CHECKSUM, 0);
        let value = checksum::checksum(field::slice(self.bytes(), 0, header_len));
        field::set_u16(self.bytes_mut(), CHECKSUM, value);
    }

    /// Sets the source address, updating the header checksum incrementally.
    ///
    /// Transport checksums cover the address too; callers doing NAT must
    /// update them with [`checksum::update_bytes`].
    pub fn set_src(&mut self, address: Ipv4Addr) {
        self.set_address(SRC, address);
    }

    /// Sets the destination address, updating the header checksum incrementally.
    ///
    /// See [`set_src`](Self::set_src) about transport checksums.
    pub fn set_dst(&mut self, address: Ipv4Addr) {
        self.set_address(DST, address);
    }

    fn set_address(&mut self, offset: usize, address: Ipv4Addr) {
        let old = field::bytes::<4>(self.bytes(), offset);
        let new = address.octets();
        let updated = update_bytes(self.header_checksum(), &old, &new);
        field::set_bytes(self.bytes_mut(), offset, new);
        field::set_u16(self.bytes_mut(), CHECKSUM, updated);
    }

    /// Sets the time to live, updating the header checksum incrementally.
    pub fn set_ttl(&mut self, ttl: u8) {
        let old = field::u16_at(self.bytes(), TTL);
        let new = u16::from_be_bytes([ttl, self.protocol()]);
        let updated = update_u16(self.header_checksum(), old, new);
        field::set_u16(self.bytes_mut(), TTL, new);
        field::set_u16(self.bytes_mut(), CHECKSUM, updated);
    }
}
