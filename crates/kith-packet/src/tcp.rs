//! TCP header view (RFC 9293).

use crate::{Error, PseudoHeader, field, ip_protocol};

const MIN_HEADER: usize = 20;
const DATA_OFFSET: usize = 12;
const FLAGS: usize = 13;
const CHECKSUM: usize = 16;

/// The eight TCP control bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TcpFlags(u8);

impl TcpFlags {
    /// No more data from sender.
    pub const FIN: Self = Self(0x01);
    /// Synchronize sequence numbers.
    pub const SYN: Self = Self(0x02);
    /// Reset the connection.
    pub const RST: Self = Self(0x04);
    /// Push function.
    pub const PSH: Self = Self(0x08);
    /// Acknowledgment field is significant.
    pub const ACK: Self = Self(0x10);
    /// Urgent pointer field is significant.
    pub const URG: Self = Self(0x20);
    /// ECN-Echo.
    pub const ECE: Self = Self(0x40);
    /// Congestion Window Reduced.
    pub const CWR: Self = Self(0x80);

    /// Flags from their wire byte.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// The wire byte.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A validated view of a TCP segment (the IP payload).
///
/// Invariant: `20 <= header_len <= buffer.len()`. The segment is the whole
/// buffer, so pass exactly the IP payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpPacket<T> {
    buffer: T,
}

impl<T: AsRef<[u8]>> TcpPacket<T> {
    /// Validates the fixed header and the data offset against `buffer`.
    pub fn new_checked(buffer: T) -> Result<Self, Error> {
        let bytes = buffer.as_ref();
        if bytes.len() < MIN_HEADER {
            return Err(Error::Truncated);
        }
        let header_len = header_len(bytes);
        if header_len < MIN_HEADER {
            return Err(Error::BadHeaderLength);
        }
        if header_len > bytes.len() {
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

    /// Sequence number.
    pub fn sequence_number(&self) -> u32 {
        field::u32_at(self.bytes(), 4)
    }

    /// Acknowledgment number.
    pub fn ack_number(&self) -> u32 {
        field::u32_at(self.bytes(), 8)
    }

    /// Data offset field, in 32-bit words.
    pub fn data_offset(&self) -> u8 {
        field::u8_at(self.bytes(), DATA_OFFSET) >> 4
    }

    /// Header length in bytes, options included.
    pub fn header_len(&self) -> usize {
        header_len(self.bytes())
    }

    /// Control bits.
    pub fn flags(&self) -> TcpFlags {
        TcpFlags(field::u8_at(self.bytes(), FLAGS))
    }

    /// Receive window.
    pub fn window(&self) -> u16 {
        field::u16_at(self.bytes(), 14)
    }

    /// Stored checksum.
    pub fn checksum(&self) -> u16 {
        field::u16_at(self.bytes(), CHECKSUM)
    }

    /// Urgent pointer.
    pub fn urgent_pointer(&self) -> u16 {
        field::u16_at(self.bytes(), 18)
    }

    /// Options bytes between the fixed header and the payload.
    pub fn options(&self) -> &[u8] {
        field::slice(self.bytes(), MIN_HEADER, self.header_len())
    }

    /// The payload after the header.
    pub fn payload(&self) -> &[u8] {
        self.bytes().get(self.header_len()..).unwrap_or(&[])
    }

    /// Whether the checksum is valid.
    pub fn verify_checksum(&self, pseudo_header: &PseudoHeader) -> bool {
        pseudo_header.checksum(ip_protocol::TCP, self.bytes()) == 0
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]>> TcpPacket<T> {
    /// Computes and stores the checksum.
    pub fn fill_checksum(&mut self, pseudo_header: &PseudoHeader) {
        field::set_u16(self.buffer.as_mut(), CHECKSUM, 0);
        let value = pseudo_header.checksum(ip_protocol::TCP, self.bytes());
        field::set_u16(self.buffer.as_mut(), CHECKSUM, value);
    }
}

fn header_len(bytes: &[u8]) -> usize {
    usize::from(field::u8_at(bytes, DATA_OFFSET) >> 4).saturating_mul(4)
}
