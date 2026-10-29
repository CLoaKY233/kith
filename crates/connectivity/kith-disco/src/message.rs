//! The three disco messages and their plaintext encoding.
//!
//! The encoding is hand-written rather than postcard because it is tiny, fixed-layout and must
//! be byte-stable forever: it is what gets sealed, so changing it is a protocol break.
//!
//! ```text
//! Ping:        0x01 | tx_id (12) | sender NodeId (32) | zero padding (any length)
//! Pong:        0x02 | tx_id (12) | address
//! CallMeMaybe: 0x03 | count (u8, at most 32) | address * count
//! address:     0x04 | IPv4 (4) | port (u16 BE)   or   0x06 | IPv6 (16) | port (u16 BE)
//! ```
//!
//! IPv6 flow labels and scope IDs are not carried: they are meaningless on the remote host.

use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use kith_types::NodeId;

/// The most endpoints a [`CallMeMaybe`] may carry.
pub const MAX_CALL_ME_MAYBE_ENDPOINTS: usize = 32;

/// The length of a transaction ID, as in Tailscale's disco.
pub const TX_ID_LEN: usize = 12;

/// A transaction ID linking a [`Pong`] to its [`Ping`].
pub type TxId = [u8; TX_ID_LEN];

/// The largest possible encoded message: a ping padded to probe a path's size. Larger than
/// a full [`CallMeMaybe`] of IPv6 addresses.
pub const MAX_PLAINTEXT_LEN: usize = 1500;

/// The encoded length of a [`Ping`] without padding.
pub const PING_LEN: usize = 1 + TX_ID_LEN + 32;

const _: () = assert!(2 + MAX_CALL_ME_MAYBE_ENDPOINTS * ADDRESS_V6_LEN <= MAX_PLAINTEXT_LEN);

const KIND_PING: u8 = 0x01;
const KIND_PONG: u8 = 0x02;
const KIND_CALL_ME_MAYBE: u8 = 0x03;
const FAMILY_V4: u8 = 0x04;
const FAMILY_V6: u8 = 0x06;
const ADDRESS_V6_LEN: usize = 1 + 16 + 2;

/// A probe asking the receiver to answer with a [`Pong`] on the same path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ping {
    /// Random per-ping identifier, echoed in the pong.
    pub tx_id: TxId,
    /// The node sending the ping.
    pub sender: NodeId,
    /// Zero bytes after the message, to make it a given size: a probe of how big a packet
    /// the path carries.
    pub padding: u16,
}

/// The answer to a [`Ping`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pong {
    /// The pinged transaction ID.
    pub tx_id: TxId,
    /// The source address the ping was seen from: the pinger's reflexive endpoint.
    pub observed: SocketAddr,
}

/// "Ping me at these endpoints now": sent over the relay to start a simultaneous hole punch.
///
/// Invariant: at most [`MAX_CALL_ME_MAYBE_ENDPOINTS`] endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallMeMaybe {
    endpoints: Vec<SocketAddr>,
}

impl CallMeMaybe {
    /// Builds a call-me-maybe, or `None` if there are more than
    /// [`MAX_CALL_ME_MAYBE_ENDPOINTS`] endpoints.
    pub fn new(endpoints: Vec<SocketAddr>) -> Option<Self> {
        (endpoints.len() <= MAX_CALL_ME_MAYBE_ENDPOINTS).then_some(Self { endpoints })
    }

    /// The endpoints the sender can be reached at.
    pub fn endpoints(&self) -> &[SocketAddr] {
        &self.endpoints
    }
}

/// Any disco message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// See [`Ping`].
    Ping(Ping),
    /// See [`Pong`].
    Pong(Pong),
    /// See [`CallMeMaybe`].
    CallMeMaybe(CallMeMaybe),
}

/// Why plaintext bytes are not a valid message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    /// The input ended early.
    #[error("message truncated")]
    Truncated,
    /// The message kind byte is unknown.
    #[error("unknown message kind {0:#04x}")]
    UnknownKind(u8),
    /// The address family byte is unknown.
    #[error("unknown address family {0:#04x}")]
    UnknownFamily(u8),
    /// A call-me-maybe declared too many endpoints.
    #[error("too many endpoints: {0}")]
    TooManyEndpoints(u8),
    /// Bytes were left after the message.
    #[error("trailing bytes after message")]
    TrailingBytes,
}

impl Message {
    /// Appends the plaintext encoding to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Ping(ping) => {
                out.push(KIND_PING);
                out.extend_from_slice(&ping.tx_id);
                out.extend_from_slice(ping.sender.as_bytes());
                out.resize(out.len().saturating_add(usize::from(ping.padding)), 0);
            }
            Self::Pong(pong) => {
                out.push(KIND_PONG);
                out.extend_from_slice(&pong.tx_id);
                encode_address(pong.observed, out);
            }
            Self::CallMeMaybe(call) => {
                out.push(KIND_CALL_ME_MAYBE);
                // The constructor caps the count at 32, so this never saturates.
                out.push(u8::try_from(call.endpoints.len()).unwrap_or(u8::MAX));
                for endpoint in &call.endpoints {
                    encode_address(*endpoint, out);
                }
            }
        }
    }

    /// Parses a complete plaintext message, rejecting trailing bytes.
    pub fn parse(input: &[u8]) -> Result<Self, ParseError> {
        let mut reader = Reader(input);
        let message = match reader.byte()? {
            KIND_PING => {
                let tx_id = reader.array()?;
                let sender = NodeId::from_bytes(reader.array()?);
                let rest = core::mem::take(&mut reader.0);
                if rest.iter().any(|byte| *byte != 0) {
                    return Err(ParseError::TrailingBytes);
                }
                let padding = u16::try_from(rest.len()).map_err(|_| ParseError::TrailingBytes)?;
                Self::Ping(Ping {
                    tx_id,
                    sender,
                    padding,
                })
            }
            KIND_PONG => Self::Pong(Pong {
                tx_id: reader.array()?,
                observed: reader.address()?,
            }),
            KIND_CALL_ME_MAYBE => {
                let count = reader.byte()?;
                let len = usize::from(count);
                if len > MAX_CALL_ME_MAYBE_ENDPOINTS {
                    return Err(ParseError::TooManyEndpoints(count));
                }
                let mut endpoints = Vec::with_capacity(len);
                for _ in 0..len {
                    endpoints.push(reader.address()?);
                }
                Self::CallMeMaybe(CallMeMaybe { endpoints })
            }
            other => return Err(ParseError::UnknownKind(other)),
        };
        if reader.0.is_empty() {
            Ok(message)
        } else {
            Err(ParseError::TrailingBytes)
        }
    }
}

fn encode_address(address: SocketAddr, out: &mut Vec<u8>) {
    match address.ip() {
        IpAddr::V4(ip) => {
            out.push(FAMILY_V4);
            out.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            out.push(FAMILY_V6);
            out.extend_from_slice(&ip.octets());
        }
    }
    out.extend_from_slice(&address.port().to_be_bytes());
}

/// A cursor over untrusted bytes; every read is bounds-checked.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, ParseError> {
        let (&first, rest) = self.0.split_first().ok_or(ParseError::Truncated)?;
        self.0 = rest;
        Ok(first)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ParseError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(ParseError::Truncated)?;
        self.0 = rest;
        Ok(*head)
    }

    fn address(&mut self) -> Result<SocketAddr, ParseError> {
        let ip = match self.byte()? {
            FAMILY_V4 => IpAddr::V4(Ipv4Addr::from(self.array::<4>()?)),
            FAMILY_V6 => IpAddr::V6(Ipv6Addr::from(self.array::<16>()?)),
            other => return Err(ParseError::UnknownFamily(other)),
        };
        let port = u16::from_be_bytes(self.array()?);
        Ok(SocketAddr::new(ip, port))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::net::SocketAddr;

    use kith_types::NodeId;
    use proptest::prelude::*;

    use super::{CallMeMaybe, MAX_PLAINTEXT_LEN, Message, ParseError, Ping, Pong};

    fn round_trip(message: &Message) -> Vec<u8> {
        let mut out = Vec::new();
        message.encode(&mut out);
        assert_eq!(Message::parse(&out).as_ref(), Ok(message));
        out
    }

    #[test]
    fn every_kind_round_trips_with_stable_address_layout() {
        round_trip(&Message::Ping(Ping {
            tx_id: [1; 12],
            sender: NodeId::from_bytes([2; 32]),
            padding: 0,
        }));
        let padded = round_trip(&Message::Ping(Ping {
            tx_id: [1; 12],
            sender: NodeId::from_bytes([2; 32]),
            padding: 1000,
        }));
        assert_eq!(padded.len(), super::PING_LEN + 1000);
        let mut dirty = padded;
        if let Some(last) = dirty.last_mut() {
            *last = 1;
        }
        assert_eq!(Message::parse(&dirty), Err(ParseError::TrailingBytes));
        let v4: SocketAddr = "192.0.2.1:41641".parse().unwrap();
        let v6: SocketAddr = "[2001:db8::1]:41641".parse().unwrap();
        let pong = round_trip(&Message::Pong(Pong {
            tx_id: [3; 12],
            observed: v4,
        }));
        assert_eq!(&pong[13..], &[4, 192, 0, 2, 1, 0xa2, 0xa9]);
        let full = CallMeMaybe::new(vec![v6; 32]).unwrap();
        assert!(round_trip(&Message::CallMeMaybe(full)).len() <= MAX_PLAINTEXT_LEN);
    }

    #[test]
    fn oversized_or_malformed_messages_are_rejected() {
        let v4: SocketAddr = "192.0.2.1:1".parse().unwrap();
        assert!(CallMeMaybe::new(vec![v4; 33]).is_none());
        assert_eq!(
            Message::parse(&[3, 33]),
            Err(ParseError::TooManyEndpoints(33))
        );
        assert_eq!(Message::parse(&[]), Err(ParseError::Truncated));
        assert_eq!(Message::parse(&[9]), Err(ParseError::UnknownKind(9)));
        assert_eq!(
            Message::parse(&[3, 1, 5]),
            Err(ParseError::UnknownFamily(5))
        );
        assert_eq!(Message::parse(&[3, 0, 0]), Err(ParseError::TrailingBytes));
        assert_eq!(Message::parse(&[1, 0, 0]), Err(ParseError::Truncated));
    }

    proptest! {
        #[test]
        fn parse_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..700)) {
            if let Ok(message) = Message::parse(&bytes) {
                let mut out = Vec::new();
                message.encode(&mut out);
                prop_assert_eq!(out, bytes);
            }
        }
    }
}
