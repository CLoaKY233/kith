//! Sans-IO NAT-PMP (RFC 6886) and PCP (RFC 6887) messages.
//!
//! Only UDP mappings are modelled; kith never maps TCP.

#![warn(clippy::arithmetic_side_effects)]

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The port both protocols listen on (RFC 6886 §3.2, RFC 6887 §19.1).
pub(crate) const SERVER_PORT: u16 = 5351;

const NATPMP_VERSION: u8 = 0;
const PCP_VERSION: u8 = 2;
const NATPMP_OP_EXTERNAL_ADDRESS: u8 = 0;
const NATPMP_OP_MAP_UDP: u8 = 1;
const RESPONSE_BIT: u8 = 0x80;
const PCP_OP_MAP: u8 = 1;
const PROTOCOL_UDP: u8 = 17;
const PCP_MAP_LENGTH: usize = 60;
/// RFC 6887 §7: messages are at most 1100 bytes.
const PCP_MAX_LENGTH: usize = 1100;

/// PCP result code for an unsupported version (RFC 6887 §7.4).
pub(crate) const PCP_UNSUPPORTED_VERSION: u8 = 1;
/// NAT-PMP result code for an unsupported version (RFC 6886 §3.5).
pub(crate) const NATPMP_UNSUPPORTED_VERSION: u16 = 1;

/// A message failed strict parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CodecError {
    #[error("message has the wrong length")]
    Length,
    #[error("unexpected version {0}")]
    Version(u8),
    #[error("unexpected opcode {0}")]
    Opcode(u8),
    #[error("unexpected protocol {0}")]
    Protocol(u8),
}

/// NAT-PMP external-address response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NatPmpAddressResponse {
    pub(crate) result: u16,
    pub(crate) epoch: u32,
    pub(crate) external: Ipv4Addr,
}

/// NAT-PMP UDP map request; lifetime 0 deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NatPmpMapRequest {
    pub(crate) internal_port: u16,
    pub(crate) suggested_external_port: u16,
    pub(crate) lifetime: u32,
}

/// NAT-PMP UDP map response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NatPmpMapResponse {
    pub(crate) result: u16,
    pub(crate) epoch: u32,
    pub(crate) internal_port: u16,
    pub(crate) external_port: u16,
    pub(crate) lifetime: u32,
}

/// PCP MAP request for UDP; lifetime 0 deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PcpMapRequest {
    pub(crate) lifetime: u32,
    pub(crate) client: IpAddr,
    pub(crate) nonce: [u8; 12],
    pub(crate) internal_port: u16,
    pub(crate) suggested_external_port: u16,
    pub(crate) suggested_external: IpAddr,
}

/// PCP MAP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PcpMapResponse {
    pub(crate) result: u8,
    pub(crate) lifetime: u32,
    pub(crate) epoch: u32,
    pub(crate) nonce: [u8; 12],
    pub(crate) internal_port: u16,
    pub(crate) external_port: u16,
    pub(crate) external: IpAddr,
}

pub(crate) const fn encode_natpmp_address_request() -> [u8; 2] {
    [NATPMP_VERSION, NATPMP_OP_EXTERNAL_ADDRESS]
}

#[cfg(test)]
pub(crate) fn parse_natpmp_address_request(bytes: &[u8]) -> Result<(), CodecError> {
    match bytes {
        [NATPMP_VERSION, NATPMP_OP_EXTERNAL_ADDRESS] => Ok(()),
        [NATPMP_VERSION, op] => Err(CodecError::Opcode(*op)),
        [version, _] => Err(CodecError::Version(*version)),
        _ => Err(CodecError::Length),
    }
}

impl NatPmpAddressResponse {
    #[cfg(test)]
    pub(crate) fn encode(&self) -> [u8; 12] {
        let mut out = [0; 12];
        let (header, rest) = out.split_at_mut(2);
        header.copy_from_slice(&[NATPMP_VERSION, RESPONSE_BIT | NATPMP_OP_EXTERNAL_ADDRESS]);
        let mut writer = Writer(rest);
        writer.put(&self.result.to_be_bytes());
        writer.put(&self.epoch.to_be_bytes());
        writer.put(&self.external.octets());
        out
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut reader = natpmp_header(bytes, RESPONSE_BIT | NATPMP_OP_EXTERNAL_ADDRESS, 12)?;
        Ok(Self {
            result: reader.u16()?,
            epoch: reader.u32()?,
            external: Ipv4Addr::from(reader.array::<4>()?),
        })
    }
}

impl NatPmpMapRequest {
    pub(crate) fn encode(self) -> [u8; 12] {
        let mut out = [0; 12];
        let mut writer = Writer(&mut out);
        writer.put(&[NATPMP_VERSION, NATPMP_OP_MAP_UDP, 0, 0]);
        writer.put(&self.internal_port.to_be_bytes());
        writer.put(&self.suggested_external_port.to_be_bytes());
        writer.put(&self.lifetime.to_be_bytes());
        out
    }

    #[cfg(test)]
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut reader = natpmp_header(bytes, NATPMP_OP_MAP_UDP, 12)?;
        reader.u16()?;
        Ok(Self {
            internal_port: reader.u16()?,
            suggested_external_port: reader.u16()?,
            lifetime: reader.u32()?,
        })
    }
}

impl NatPmpMapResponse {
    #[cfg(test)]
    pub(crate) fn encode(&self) -> [u8; 16] {
        let mut out = [0; 16];
        let mut writer = Writer(&mut out);
        writer.put(&[NATPMP_VERSION, RESPONSE_BIT | NATPMP_OP_MAP_UDP]);
        writer.put(&self.result.to_be_bytes());
        writer.put(&self.epoch.to_be_bytes());
        writer.put(&self.internal_port.to_be_bytes());
        writer.put(&self.external_port.to_be_bytes());
        writer.put(&self.lifetime.to_be_bytes());
        out
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut reader = natpmp_header(bytes, RESPONSE_BIT | NATPMP_OP_MAP_UDP, 16)?;
        Ok(Self {
            result: reader.u16()?,
            epoch: reader.u32()?,
            internal_port: reader.u16()?,
            external_port: reader.u16()?,
            lifetime: reader.u32()?,
        })
    }
}

/// Recognises a NAT-PMP server rejecting a PCP request (RFC 6887 §9).
pub(crate) fn is_natpmp_unsupported_version(bytes: &[u8]) -> bool {
    matches!(bytes, [NATPMP_VERSION, op, result @ .., ] if op & RESPONSE_BIT != 0
        && result.get(..2) == Some(&NATPMP_UNSUPPORTED_VERSION.to_be_bytes()[..]))
}

impl PcpMapRequest {
    pub(crate) fn encode(&self) -> [u8; PCP_MAP_LENGTH] {
        let mut out = [0; PCP_MAP_LENGTH];
        let mut writer = Writer(&mut out);
        writer.put(&[PCP_VERSION, PCP_OP_MAP, 0, 0]);
        writer.put(&self.lifetime.to_be_bytes());
        writer.put(&to_v6(self.client).octets());
        writer.put(&self.nonce);
        writer.put(&[PROTOCOL_UDP, 0, 0, 0]);
        writer.put(&self.internal_port.to_be_bytes());
        writer.put(&self.suggested_external_port.to_be_bytes());
        writer.put(&to_v6(self.suggested_external).octets());
        out
    }

    #[cfg(test)]
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut reader = pcp_header(bytes, PCP_OP_MAP)?;
        reader.array::<2>()?;
        let lifetime = reader.u32()?;
        let client = from_v6(reader.array()?);
        let (nonce, internal_port, suggested_external_port, suggested_external) =
            map_payload(&mut reader)?;
        Ok(Self {
            lifetime,
            client,
            nonce,
            internal_port,
            suggested_external_port,
            suggested_external,
        })
    }
}

impl PcpMapResponse {
    #[cfg(test)]
    pub(crate) fn encode(&self) -> [u8; PCP_MAP_LENGTH] {
        let mut out = [0; PCP_MAP_LENGTH];
        let mut writer = Writer(&mut out);
        writer.put(&[PCP_VERSION, RESPONSE_BIT | PCP_OP_MAP, 0, self.result]);
        writer.put(&self.lifetime.to_be_bytes());
        writer.put(&self.epoch.to_be_bytes());
        writer.put(&[0; 12]);
        writer.put(&self.nonce);
        writer.put(&[PROTOCOL_UDP, 0, 0, 0]);
        writer.put(&self.internal_port.to_be_bytes());
        writer.put(&self.external_port.to_be_bytes());
        writer.put(&to_v6(self.external).octets());
        out
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut reader = pcp_header(bytes, RESPONSE_BIT | PCP_OP_MAP)?;
        let [_, result] = reader.array()?;
        let lifetime = reader.u32()?;
        let epoch = reader.u32()?;
        reader.array::<12>()?;
        let (nonce, internal_port, external_port, external) = map_payload(&mut reader)?;
        Ok(Self {
            result,
            lifetime,
            epoch,
            nonce,
            internal_port,
            external_port,
            external,
        })
    }
}

fn natpmp_header(bytes: &[u8], opcode: u8, length: usize) -> Result<Reader<'_>, CodecError> {
    if bytes.len() != length {
        return Err(CodecError::Length);
    }
    let mut reader = Reader(bytes);
    let [version, op] = reader.array()?;
    if version != NATPMP_VERSION {
        return Err(CodecError::Version(version));
    }
    if op != opcode {
        return Err(CodecError::Opcode(op));
    }
    Ok(reader)
}

/// Options may follow the MAP payload in 4-byte units (RFC 6887 §7.3); they are ignored.
fn pcp_header(bytes: &[u8], opcode: u8) -> Result<Reader<'_>, CodecError> {
    if bytes.len() < PCP_MAP_LENGTH
        || bytes.len() > PCP_MAX_LENGTH
        || !bytes.len().is_multiple_of(4)
    {
        return Err(CodecError::Length);
    }
    let mut reader = Reader(bytes);
    let [version, op] = reader.array()?;
    if version != PCP_VERSION {
        return Err(CodecError::Version(version));
    }
    if op != opcode {
        return Err(CodecError::Opcode(op));
    }
    Ok(reader)
}

fn map_payload(reader: &mut Reader<'_>) -> Result<([u8; 12], u16, u16, IpAddr), CodecError> {
    let nonce = reader.array()?;
    let [protocol, ..] = reader.array::<4>()?;
    if protocol != PROTOCOL_UDP {
        return Err(CodecError::Protocol(protocol));
    }
    Ok((
        nonce,
        reader.u16()?,
        reader.u16()?,
        from_v6(reader.array()?),
    ))
}

fn to_v6(address: IpAddr) -> Ipv6Addr {
    match address {
        IpAddr::V4(v4) => v4.to_ipv6_mapped(),
        IpAddr::V6(v6) => v6,
    }
}

fn from_v6(octets: [u8; 16]) -> IpAddr {
    Ipv6Addr::from(octets).to_canonical()
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(CodecError::Length)?;
        self.0 = rest;
        Ok(*head)
    }

    fn u16(&mut self) -> Result<u16, CodecError> {
        self.array().map(u16::from_be_bytes)
    }

    fn u32(&mut self) -> Result<u32, CodecError> {
        self.array().map(u32::from_be_bytes)
    }
}

/// Writes into a fixed buffer whose size the caller has already matched to the message.
struct Writer<'a>(&'a mut [u8]);

impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) {
        let buffer = core::mem::take(&mut self.0);
        if let Some((head, rest)) = buffer.split_at_mut_checked(bytes.len()) {
            head.copy_from_slice(bytes);
            self.0 = rest;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const NONCE: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

    #[test]
    fn natpmp_messages_match_rfc_6886_layout() {
        assert_eq!(encode_natpmp_address_request(), [0, 0]);
        let address = NatPmpAddressResponse {
            result: 0,
            epoch: 7,
            external: Ipv4Addr::new(203, 0, 113, 9),
        };
        assert_eq!(address.encode(), [0, 128, 0, 0, 0, 0, 0, 7, 203, 0, 113, 9]);
        let request = NatPmpMapRequest {
            internal_port: 41641,
            suggested_external_port: 41641,
            lifetime: 7200,
        };
        assert_eq!(
            request.encode(),
            [0, 1, 0, 0, 0xa2, 0xa9, 0xa2, 0xa9, 0, 0, 0x1c, 0x20]
        );
        let response = NatPmpMapResponse {
            result: 0,
            epoch: 7,
            internal_port: 41641,
            external_port: 50000,
            lifetime: 3600,
        };
        assert_eq!(
            response.encode(),
            [
                0, 129, 0, 0, 0, 0, 0, 7, 0xa2, 0xa9, 0xc3, 0x50, 0, 0, 0x0e, 0x10
            ]
        );
        assert!(is_natpmp_unsupported_version(&[0, 0x81, 0, 1, 0, 0, 0, 0]));
    }

    #[test]
    fn pcp_map_messages_match_rfc_6887_layout() {
        let request = PcpMapRequest {
            lifetime: 7200,
            client: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            nonce: NONCE,
            internal_port: 41641,
            suggested_external_port: 0,
            suggested_external: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        let mut expected = vec![2, 1, 0, 0, 0, 0, 0x1c, 0x20];
        expected.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 168, 1, 10]);
        expected.extend(NONCE);
        expected.extend([17, 0, 0, 0, 0xa2, 0xa9, 0, 0]);
        expected.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0]);
        assert_eq!(request.encode().as_slice(), expected);

        let response = PcpMapResponse {
            result: 0,
            lifetime: 3600,
            epoch: 99,
            nonce: NONCE,
            internal_port: 41641,
            external_port: 50000,
            external: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
        };
        let mut expected = vec![2, 0x81, 0, 0, 0, 0, 0x0e, 0x10, 0, 0, 0, 99];
        expected.extend([0; 12]);
        expected.extend(NONCE);
        expected.extend([17, 0, 0, 0, 0xa2, 0xa9, 0xc3, 0x50]);
        expected.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 203, 0, 113, 9]);
        assert_eq!(response.encode().as_slice(), expected);
    }

    fn ip() -> impl Strategy<Value = IpAddr> {
        prop_oneof![
            any::<[u8; 4]>().prop_map(|o| IpAddr::V4(o.into())),
            // Mapped IPv6 would canonicalise to IPv4, so exclude it from round-trips.
            any::<[u8; 16]>()
                .prop_map(|o| IpAddr::V6(o.into()))
                .prop_filter("not v4-mapped", |a| a.to_canonical() == *a),
        ]
    }

    proptest! {
        #[test]
        fn parsers_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..128)) {
            let _ = parse_natpmp_address_request(&bytes);
            let _ = NatPmpAddressResponse::parse(&bytes);
            let _ = NatPmpMapRequest::parse(&bytes);
            let _ = NatPmpMapResponse::parse(&bytes);
            let _ = PcpMapRequest::parse(&bytes);
            let _ = PcpMapResponse::parse(&bytes);
            let _ = is_natpmp_unsupported_version(&bytes);
        }

        #[test]
        fn encode_then_parse_round_trips(
            words in any::<(u16, u16, u32, u32)>(),
            byte in any::<u8>(),
            nonce in any::<[u8; 12]>(),
            v4 in any::<[u8; 4]>(),
            first in ip(),
            second in ip(),
        ) {
            let (a, b, x, y) = words;
            parse_natpmp_address_request(&encode_natpmp_address_request()).unwrap();
            let address = NatPmpAddressResponse { result: a, epoch: x, external: v4.into() };
            prop_assert_eq!(NatPmpAddressResponse::parse(&address.encode()), Ok(address));
            let request = NatPmpMapRequest { internal_port: a, suggested_external_port: b, lifetime: x };
            prop_assert_eq!(NatPmpMapRequest::parse(&request.encode()), Ok(request));
            let response = NatPmpMapResponse { result: a, epoch: x, internal_port: b, external_port: a, lifetime: y };
            prop_assert_eq!(NatPmpMapResponse::parse(&response.encode()), Ok(response));
            let request = PcpMapRequest {
                lifetime: x, client: first, nonce, internal_port: a,
                suggested_external_port: b, suggested_external: second,
            };
            prop_assert_eq!(PcpMapRequest::parse(&request.encode()), Ok(request));
            let response = PcpMapResponse {
                result: byte, lifetime: x, epoch: y, nonce, internal_port: a,
                external_port: b, external: second,
            };
            prop_assert_eq!(PcpMapResponse::parse(&response.encode()), Ok(response));
        }
    }
}
