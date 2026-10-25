//! The STUN message codec: header, attribute walk, address attributes, FINGERPRINT.
//!
//! The parser trusts nothing: every offset is computed with checked arithmetic,
//! every slice is taken with `get`/`split_at_checked`, and the declared message
//! length must match the datagram exactly.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::crc32::crc32;

/// The fixed value in bytes 4..8 of every RFC 5389+ message.
const MAGIC_COOKIE: [u8; 4] = [0x21, 0x12, 0xA4, 0x42];
/// XOR-ed into the CRC to form FINGERPRINT, so a STUN CRC never matches the
/// CRC another protocol might carry at the same position (RFC 8489 §14.7).
const FINGERPRINT_XOR: u32 = 0x5354_554E;

const HEADER_LEN: usize = 20;
const ATTRIBUTE_HEADER_LEN: usize = 4;
const FINGERPRINT_ATTRIBUTE_LEN: usize = ATTRIBUTE_HEADER_LEN + 4;
/// XOR-MAPPED-ADDRESS carrying an IPv6 address: reserved, family, port, 16 bytes.
const MAX_ADDRESS_ATTRIBUTE_LEN: usize = ATTRIBUTE_HEADER_LEN + 20;

const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS_RESPONSE: u16 = 0x0101;

const ATTRIBUTE_MAPPED_ADDRESS: u16 = 0x0001;
const ATTRIBUTE_MESSAGE_INTEGRITY: u16 = 0x0008;
const ATTRIBUTE_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const ATTRIBUTE_MESSAGE_INTEGRITY_SHA256: u16 = 0x001C;
const ATTRIBUTE_FINGERPRINT: u16 = 0x8028;
/// Attribute types below this must be understood or the message rejected.
const COMPREHENSION_OPTIONAL_START: u16 = 0x8000;

const FAMILY_IPV4: u8 = 0x01;
const FAMILY_IPV6: u8 = 0x02;

/// Length of an encoded binding request: header plus FINGERPRINT.
pub(crate) const REQUEST_LEN: usize = HEADER_LEN + FINGERPRINT_ATTRIBUTE_LEN;
/// Largest encoded binding response (an IPv6 XOR-MAPPED-ADDRESS plus FINGERPRINT).
pub const MAX_RESPONSE_LEN: usize =
    HEADER_LEN + MAX_ADDRESS_ATTRIBUTE_LEN + FINGERPRINT_ATTRIBUTE_LEN;

/// The 96-bit transaction ID that pairs a response with its request.
///
/// Callers must fill it from a cryptographically secure RNG: an off-path
/// attacker who can guess it can forge our reflexive address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransactionId(pub [u8; 12]);

/// Why a packet was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StunError {
    /// The header is malformed: too short, wrong cookie, top bits set, or a
    /// declared length that does not match the datagram.
    #[error("not a STUN message")]
    NotStun,
    /// A well-formed STUN message of a type this call does not handle.
    #[error("unexpected STUN message type {0:#06x}")]
    UnexpectedType(u16),
    /// An attribute header or value runs past the end of the message.
    #[error("truncated STUN attribute")]
    TruncatedAttribute,
    /// A comprehension-required attribute this implementation does not know.
    #[error("unknown comprehension-required STUN attribute {0:#06x}")]
    UnknownRequiredAttribute(u16),
    /// FINGERPRINT is malformed, not last, or does not match the message.
    #[error("bad STUN fingerprint")]
    BadFingerprint,
    /// A (XOR-)MAPPED-ADDRESS has an unknown family or the wrong length.
    #[error("malformed STUN address attribute")]
    BadAddress,
    /// A binding response carries neither XOR-MAPPED-ADDRESS nor MAPPED-ADDRESS.
    #[error("STUN binding response has no mapped address")]
    MissingAddress,
}

/// Cheap check for whether a datagram looks like STUN.
///
/// Meant for the UDP demultiplexer: it checks the magic cookie, the two zero
/// top bits and that the declared length is 4-aligned and matches the
/// datagram. WireGuard messages start with type bytes `1..=4` followed by
/// three zero bytes, so they never carry the cookie.
pub fn is_stun(packet: &[u8]) -> bool {
    parse_header(packet).is_ok()
}

/// Encodes a binding request (with FINGERPRINT) into `out` and returns it.
pub(crate) fn encode_binding_request(txid: TransactionId, out: &mut [u8; REQUEST_LEN]) -> &[u8] {
    let mut writer = Writer::new(out);
    writer.header(BINDING_REQUEST, txid, 0);
    writer.finish_with_fingerprint()
}

/// Encodes a binding success response telling the client its `observed`
/// source address, as XOR-MAPPED-ADDRESS plus FINGERPRINT.
pub fn encode_binding_response(
    txid: TransactionId,
    observed: SocketAddr,
    out: &mut [u8; MAX_RESPONSE_LEN],
) -> &[u8] {
    let port =
        (observed.port() ^ u16::from_be_bytes([MAGIC_COOKIE[0], MAGIC_COOKIE[1]])).to_be_bytes();
    let mut writer = Writer::new(out);
    match observed.ip() {
        IpAddr::V4(ip) => {
            let mut address = ip.octets();
            xor_in_place(&mut address, &MAGIC_COOKIE);
            writer.header(BINDING_SUCCESS_RESPONSE, txid, 12);
            writer.put(&ATTRIBUTE_XOR_MAPPED_ADDRESS.to_be_bytes());
            writer.put(&8_u16.to_be_bytes());
            writer.put(&[0, FAMILY_IPV4]);
            writer.put(&port);
            writer.put(&address);
        }
        IpAddr::V6(ip) => {
            let mut address = ip.octets();
            xor_in_place(&mut address, &ipv6_mask(txid));
            writer.header(BINDING_SUCCESS_RESPONSE, txid, 24);
            writer.put(&ATTRIBUTE_XOR_MAPPED_ADDRESS.to_be_bytes());
            writer.put(&20_u16.to_be_bytes());
            writer.put(&[0, FAMILY_IPV6]);
            writer.put(&port);
            writer.put(&address);
        }
    }
    writer.finish_with_fingerprint()
}

/// Parses a binding request, as a STUN server, returning its transaction ID.
pub fn parse_binding_request(packet: &[u8]) -> Result<TransactionId, StunError> {
    let message = parse_message(packet)?;
    if message.kind != BINDING_REQUEST {
        return Err(StunError::UnexpectedType(message.kind));
    }
    Ok(message.txid)
}

/// Parses a binding success response, returning its transaction ID and the
/// reflexive address it reports.
///
/// XOR-MAPPED-ADDRESS is preferred; MAPPED-ADDRESS is the fallback for old
/// RFC 3489 servers. The caller must still check the transaction ID.
pub(crate) fn parse_binding_response(
    packet: &[u8],
) -> Result<(TransactionId, SocketAddr), StunError> {
    let message = parse_message(packet)?;
    if message.kind != BINDING_SUCCESS_RESPONSE {
        return Err(StunError::UnexpectedType(message.kind));
    }
    let address = match (message.xor_mapped_address, message.mapped_address) {
        (Some(value), _) => parse_address(value, Some(message.txid))?,
        (None, Some(value)) => parse_address(value, None)?,
        (None, None) => return Err(StunError::MissingAddress),
    };
    Ok((message.txid, address))
}

/// A header-validated message with the attributes we care about located.
struct Message<'a> {
    kind: u16,
    txid: TransactionId,
    xor_mapped_address: Option<&'a [u8]>,
    mapped_address: Option<&'a [u8]>,
}

/// Validates the header, returning the message type, transaction ID and the
/// attribute bytes.
fn parse_header(packet: &[u8]) -> Result<(u16, TransactionId, &[u8]), StunError> {
    let (header, attributes) = packet
        .split_first_chunk::<HEADER_LEN>()
        .ok_or(StunError::NotStun)?;
    let [t0, t1, l0, l1, c0, c1, c2, c3, txid @ ..] = *header;
    let kind = u16::from_be_bytes([t0, t1]);
    let length = usize::from(u16::from_be_bytes([l0, l1]));
    let well_formed = kind & 0xC000 == 0
        && [c0, c1, c2, c3] == MAGIC_COOKIE
        && length % 4 == 0
        && length == attributes.len();
    if well_formed {
        Ok((kind, TransactionId(txid), attributes))
    } else {
        Err(StunError::NotStun)
    }
}

fn parse_message(packet: &[u8]) -> Result<Message<'_>, StunError> {
    let (kind, txid, attributes) = parse_header(packet)?;
    let mut message = Message {
        kind,
        txid,
        xor_mapped_address: None,
        mapped_address: None,
    };
    // After MESSAGE-INTEGRITY only FINGERPRINT may follow; anything else is
    // ignored (RFC 8489 §14.5).
    let mut after_integrity = false;
    let mut rest = attributes;
    while !rest.is_empty() {
        let offset = packet
            .len()
            .checked_sub(rest.len())
            .ok_or(StunError::TruncatedAttribute)?;
        let (attribute_header, after_header) = rest
            .split_first_chunk::<ATTRIBUTE_HEADER_LEN>()
            .ok_or(StunError::TruncatedAttribute)?;
        let [a0, a1, v0, v1] = *attribute_header;
        let attribute_type = u16::from_be_bytes([a0, a1]);
        let value_len = usize::from(u16::from_be_bytes([v0, v1]));
        let (value, after_value) = after_header
            .split_at_checked(value_len)
            .ok_or(StunError::TruncatedAttribute)?;
        // Values are padded to a 4-byte boundary; the padding is not counted in the length.
        let padding = value_len.wrapping_neg() % 4;
        let (_, next) = after_value
            .split_at_checked(padding)
            .ok_or(StunError::TruncatedAttribute)?;
        rest = next;

        match attribute_type {
            ATTRIBUTE_FINGERPRINT => {
                if !rest.is_empty() {
                    return Err(StunError::BadFingerprint);
                }
                verify_fingerprint(packet, offset, value)?;
            }
            _ if after_integrity => {}
            ATTRIBUTE_XOR_MAPPED_ADDRESS => {
                message.xor_mapped_address.get_or_insert(value);
            }
            ATTRIBUTE_MAPPED_ADDRESS => {
                message.mapped_address.get_or_insert(value);
            }
            ATTRIBUTE_MESSAGE_INTEGRITY | ATTRIBUTE_MESSAGE_INTEGRITY_SHA256 => {
                after_integrity = true;
            }
            unknown if unknown < COMPREHENSION_OPTIONAL_START => {
                return Err(StunError::UnknownRequiredAttribute(unknown));
            }
            _ => {}
        }
    }
    Ok(message)
}

/// Checks a FINGERPRINT value against the CRC of everything before it.
fn verify_fingerprint(packet: &[u8], offset: usize, value: &[u8]) -> Result<(), StunError> {
    let covered = packet.get(..offset).ok_or(StunError::BadFingerprint)?;
    let expected = (crc32(covered) ^ FINGERPRINT_XOR).to_be_bytes();
    if value == expected {
        Ok(())
    } else {
        Err(StunError::BadFingerprint)
    }
}

/// Decodes a (XOR-)MAPPED-ADDRESS value. `xor_with` is the transaction ID for
/// XOR-MAPPED-ADDRESS, or `None` for plain MAPPED-ADDRESS.
fn parse_address(value: &[u8], xor_with: Option<TransactionId>) -> Result<SocketAddr, StunError> {
    let (prefix, address) = value
        .split_first_chunk::<4>()
        .ok_or(StunError::BadAddress)?;
    let [_reserved, family, p0, p1] = *prefix;
    let mut port = u16::from_be_bytes([p0, p1]);
    if xor_with.is_some() {
        port ^= u16::from_be_bytes([MAGIC_COOKIE[0], MAGIC_COOKIE[1]]);
    }
    let ip = match family {
        FAMILY_IPV4 => {
            let mut octets: [u8; 4] = address.try_into().map_err(|_| StunError::BadAddress)?;
            if xor_with.is_some() {
                xor_in_place(&mut octets, &MAGIC_COOKIE);
            }
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        FAMILY_IPV6 => {
            let mut octets: [u8; 16] = address.try_into().map_err(|_| StunError::BadAddress)?;
            if let Some(txid) = xor_with {
                xor_in_place(&mut octets, &ipv6_mask(txid));
            }
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        _ => return Err(StunError::BadAddress),
    };
    Ok(SocketAddr::new(ip, port))
}

/// The XOR mask for an IPv6 XOR-MAPPED-ADDRESS: magic cookie then transaction ID.
fn ipv6_mask(txid: TransactionId) -> [u8; 16] {
    let mut mask = [0_u8; 16];
    for (slot, byte) in mask
        .iter_mut()
        .zip(MAGIC_COOKIE.iter().chain(txid.0.iter()))
    {
        *slot = *byte;
    }
    mask
}

fn xor_in_place(target: &mut [u8], mask: &[u8]) {
    for (byte, mask_byte) in target.iter_mut().zip(mask) {
        *byte ^= mask_byte;
    }
}

/// Appends to a fixed buffer whose size the encoders have chosen to fit exactly.
struct Writer<'a> {
    buffer: &'a mut [u8],
    len: usize,
}

impl<'a> Writer<'a> {
    fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, len: 0 }
    }

    /// Appends `bytes`. Overflow is unreachable (golden tests pin every layout); it truncates rather than panics.
    fn put(&mut self, bytes: &[u8]) {
        let Some(end) = self.len.checked_add(bytes.len()) else {
            return;
        };
        if let Some(destination) = self.buffer.get_mut(self.len..end) {
            destination.copy_from_slice(bytes);
            self.len = end;
        }
    }

    /// Writes the header. `attributes_len` excludes FINGERPRINT, which is added here.
    fn header(&mut self, kind: u16, txid: TransactionId, attributes_len: u16) {
        let length = attributes_len.saturating_add(8);
        self.put(&kind.to_be_bytes());
        self.put(&length.to_be_bytes());
        self.put(&MAGIC_COOKIE);
        self.put(&txid.0);
    }

    /// Appends FINGERPRINT over everything written so far and returns the message.
    fn finish_with_fingerprint(mut self) -> &'a [u8] {
        // The CRC covers everything before the FINGERPRINT attribute; the
        // header length already counts it.
        let covered = self.buffer.get(..self.len).unwrap_or_default();
        let fingerprint = crc32(covered) ^ FINGERPRINT_XOR;
        self.put(&ATTRIBUTE_FINGERPRINT.to_be_bytes());
        self.put(&4_u16.to_be_bytes());
        self.put(&fingerprint.to_be_bytes());
        let Self { buffer, len } = self;
        let buffer: &'a [u8] = buffer;
        buffer.get(..len).unwrap_or_default()
    }
}
