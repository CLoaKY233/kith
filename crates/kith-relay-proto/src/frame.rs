//! The relay frame codec.
//!
//! A relay connection is a reliable, ordered byte stream (TLS, WebSocket or a QUIC stream)
//! carrying a sequence of frames:
//!
//! ```text
//! type (u8) | body length (u16, big-endian) | body (length bytes)
//! ```
//!
//! | Type   | Frame             | Body                                            |
//! |--------|-------------------|-------------------------------------------------|
//! | `0x01` | `ServerChallenge` | protocol version (u16 BE), nonce (32)           |
//! | `0x02` | `ClientHello`     | node id (32), Ed25519 signature (64)            |
//! | `0x03` | `ServerAccept`    | empty                                           |
//! | `0x04` | `ServerReject`    | reason code (u8)                                |
//! | `0x10` | `SendPacket`      | destination node id (32), payload               |
//! | `0x11` | `RecvPacket`      | source node id (32), payload                    |
//! | `0x12` | `PeerGone`        | node id (32)                                    |
//! | `0x20` | `Ping`            | data (8)                                        |
//! | `0x21` | `Pong`            | data (8)                                        |
//! | `0x22` | `Health`          | empty when healthy, else one problem code (u8)  |
//!
//! Decoding is strict: a known frame whose body is not exactly the expected shape is an
//! error, and the stream must then be closed. Unknown frame types are skipped whole, which
//! keeps the protocol forward-compatible: a newer peer may send frames this version does not
//! understand, and they are ignored rather than tearing down the connection. Unknown reason
//! and problem codes are likewise carried through rather than rejected.
//!
//! Payloads are opaque, already-encrypted WireGuard packets. The relay never sees plaintext.

use alloc::vec::Vec;

use kith_crypto::Signature;
use kith_types::NodeId;

/// Bytes in a frame header: type (1) and body length (2).
pub const HEADER_LEN: usize = 3;

/// Largest body a frame can carry: the largest value of the u16 length field.
pub const MAX_BODY_LEN: usize = 65_535;

/// Largest packet payload: [`MAX_BODY_LEN`] minus the 32-byte node id that precedes it.
pub const MAX_PAYLOAD_LEN: usize = 65_503;

/// The protocol version this crate speaks, announced in [`Frame::ServerChallenge`].
pub const PROTOCOL_VERSION: u16 = 1;

/// Bytes in a challenge nonce.
pub const NONCE_LEN: usize = 32;

const NODE_ID_LEN: usize = 32;
const SIGNATURE_LEN: usize = 64;
const PING_LEN: usize = 8;

const TYPE_SERVER_CHALLENGE: u8 = 0x01;
const TYPE_CLIENT_HELLO: u8 = 0x02;
const TYPE_SERVER_ACCEPT: u8 = 0x03;
const TYPE_SERVER_REJECT: u8 = 0x04;
const TYPE_SEND_PACKET: u8 = 0x10;
const TYPE_RECV_PACKET: u8 = 0x11;
const TYPE_PEER_GONE: u8 = 0x12;
const TYPE_PING: u8 = 0x20;
const TYPE_PONG: u8 = 0x21;
const TYPE_HEALTH: u8 = 0x22;

/// Why a relay refused a client. Unknown codes from newer relays are preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RejectReason(u8);

impl RejectReason {
    /// The client is not a member of the network this relay serves.
    pub const NOT_MEMBER: Self = Self(1);
    /// The challenge signature did not verify.
    pub const BAD_SIGNATURE: Self = Self(2);
    /// The client sent a frame that is not allowed before authentication.
    pub const PROTOCOL_VIOLATION: Self = Self(3);

    /// Wraps a wire code.
    #[must_use]
    pub const fn from_code(code: u8) -> Self {
        Self(code)
    }

    /// The wire code.
    #[must_use]
    pub const fn code(self) -> u8 {
        self.0
    }
}

/// A problem the relay reports about itself. Unknown codes from newer relays are preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HealthProblem(u8);

impl HealthProblem {
    /// The relay is overloaded and may drop packets.
    pub const OVERLOADED: Self = Self(1);
    /// The relay is shutting down; clients should move elsewhere.
    pub const SHUTTING_DOWN: Self = Self(2);

    /// Wraps a wire code.
    #[must_use]
    pub const fn from_code(code: u8) -> Self {
        Self(code)
    }

    /// The wire code.
    #[must_use]
    pub const fn code(self) -> u8 {
        self.0
    }
}

/// One relay protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Server to client, first frame: prove you hold your key by signing `nonce`.
    ServerChallenge {
        /// The relay's protocol version.
        protocol_version: u16,
        /// Fresh randomness, unique to this connection.
        nonce: [u8; NONCE_LEN],
    },
    /// Client to server: the answer to the challenge. See [`crate::auth`].
    ClientHello {
        /// The identity the client claims.
        node: NodeId,
        /// Signature by `node` over the nonce and the relay's identity.
        signature: Signature,
    },
    /// Server to client: authentication succeeded.
    ServerAccept,
    /// Server to client: authentication failed; the server closes the connection.
    ServerReject {
        /// Why the client was refused.
        reason: RejectReason,
    },
    /// Client to server: deliver `payload` to `dst`.
    SendPacket {
        /// The destination node.
        dst: NodeId,
        /// An opaque, already-encrypted packet. At most [`MAX_PAYLOAD_LEN`] bytes.
        payload: Vec<u8>,
    },
    /// Server to client: `src` sent you `payload`. `src` is the sender's authenticated identity.
    RecvPacket {
        /// The authenticated sender.
        src: NodeId,
        /// An opaque, already-encrypted packet. At most [`MAX_PAYLOAD_LEN`] bytes.
        payload: Vec<u8>,
    },
    /// Server to client: `node` is not connected to this relay.
    PeerGone {
        /// The unreachable node.
        node: NodeId,
    },
    /// Either direction: liveness probe; the peer answers with [`Frame::Pong`].
    Ping {
        /// Opaque data echoed in the reply.
        data: [u8; PING_LEN],
    },
    /// Either direction: the answer to a [`Frame::Ping`].
    Pong {
        /// The data from the ping.
        data: [u8; PING_LEN],
    },
    /// Server to client: the relay's own health.
    Health {
        /// `None` when healthy.
        problem: Option<HealthProblem>,
    },
}

/// A frame's body exceeds [`MAX_BODY_LEN`]; only packet frames can hit this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("relay frame body of {len} bytes exceeds the {MAX_BODY_LEN}-byte limit")]
pub struct FrameTooLarge {
    /// The body length that was attempted.
    pub len: usize,
}

/// A known frame type arrived with a malformed body. The stream must be closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("malformed relay frame of type {frame_type:#04x}")]
pub struct DecodeError {
    /// The type byte of the offending frame.
    pub frame_type: u8,
}

impl Frame {
    /// The type byte on the wire.
    #[must_use]
    pub const fn frame_type(&self) -> u8 {
        match self {
            Self::ServerChallenge { .. } => TYPE_SERVER_CHALLENGE,
            Self::ClientHello { .. } => TYPE_CLIENT_HELLO,
            Self::ServerAccept => TYPE_SERVER_ACCEPT,
            Self::ServerReject { .. } => TYPE_SERVER_REJECT,
            Self::SendPacket { .. } => TYPE_SEND_PACKET,
            Self::RecvPacket { .. } => TYPE_RECV_PACKET,
            Self::PeerGone { .. } => TYPE_PEER_GONE,
            Self::Ping { .. } => TYPE_PING,
            Self::Pong { .. } => TYPE_PONG,
            Self::Health { .. } => TYPE_HEALTH,
        }
    }

    /// Bytes this frame occupies on the wire, header included.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        HEADER_LEN.saturating_add(self.body_len())
    }

    fn body_len(&self) -> usize {
        match self {
            Self::ServerChallenge { .. } => 2_usize.saturating_add(NONCE_LEN),
            Self::ClientHello { .. } => NODE_ID_LEN.saturating_add(SIGNATURE_LEN),
            Self::ServerAccept | Self::Health { problem: None } => 0,
            Self::ServerReject { .. } | Self::Health { problem: Some(_) } => 1,
            Self::SendPacket { payload, .. } | Self::RecvPacket { payload, .. } => {
                NODE_ID_LEN.saturating_add(payload.len())
            }
            Self::PeerGone { .. } => NODE_ID_LEN,
            Self::Ping { .. } | Self::Pong { .. } => PING_LEN,
        }
    }

    /// Appends this frame to `out`. On error `out` is left unchanged.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), FrameTooLarge> {
        let body_len = self.body_len();
        let len = u16::try_from(body_len).map_err(|_| FrameTooLarge { len: body_len })?;
        out.reserve(HEADER_LEN.saturating_add(body_len));
        out.push(self.frame_type());
        out.extend_from_slice(&len.to_be_bytes());
        match self {
            Self::ServerChallenge {
                protocol_version,
                nonce,
            } => {
                out.extend_from_slice(&protocol_version.to_be_bytes());
                out.extend_from_slice(nonce);
            }
            Self::ClientHello { node, signature } => {
                out.extend_from_slice(node.as_bytes());
                out.extend_from_slice(signature.as_bytes());
            }
            Self::ServerAccept | Self::Health { problem: None } => {}
            Self::ServerReject { reason } => out.push(reason.code()),
            Self::SendPacket { dst: node, payload } | Self::RecvPacket { src: node, payload } => {
                out.extend_from_slice(node.as_bytes());
                out.extend_from_slice(payload);
            }
            Self::PeerGone { node } => out.extend_from_slice(node.as_bytes()),
            Self::Ping { data } | Self::Pong { data } => out.extend_from_slice(data),
            Self::Health {
                problem: Some(problem),
            } => out.push(problem.code()),
        }
        Ok(())
    }

    /// Parses one body. `Ok(None)` means the type is unknown and the frame is skipped.
    fn parse(frame_type: u8, body: &[u8]) -> Result<Option<Self>, DecodeError> {
        let mut reader = Reader {
            frame_type,
            rest: body,
        };
        let frame = match frame_type {
            TYPE_SERVER_CHALLENGE => Self::ServerChallenge {
                protocol_version: u16::from_be_bytes(reader.array()?),
                nonce: reader.array()?,
            },
            TYPE_CLIENT_HELLO => Self::ClientHello {
                node: NodeId::from_bytes(reader.array()?),
                signature: Signature::from_bytes(reader.array()?),
            },
            TYPE_SERVER_ACCEPT => Self::ServerAccept,
            TYPE_SERVER_REJECT => Self::ServerReject {
                reason: RejectReason::from_code(reader.byte()?),
            },
            TYPE_SEND_PACKET => Self::SendPacket {
                dst: NodeId::from_bytes(reader.array()?),
                payload: reader.take_rest(),
            },
            TYPE_RECV_PACKET => Self::RecvPacket {
                src: NodeId::from_bytes(reader.array()?),
                payload: reader.take_rest(),
            },
            TYPE_PEER_GONE => Self::PeerGone {
                node: NodeId::from_bytes(reader.array()?),
            },
            TYPE_PING => Self::Ping {
                data: reader.array()?,
            },
            TYPE_PONG => Self::Pong {
                data: reader.array()?,
            },
            TYPE_HEALTH => Self::Health {
                problem: if reader.rest.is_empty() {
                    None
                } else {
                    Some(HealthProblem::from_code(reader.byte()?))
                },
            },
            _ => return Ok(None),
        };
        reader.finish()?;
        Ok(Some(frame))
    }
}

/// Reads fixed-size fields from a frame body, failing on short input.
struct Reader<'a> {
    frame_type: u8,
    rest: &'a [u8],
}

impl Reader<'_> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, tail) = self.rest.split_first_chunk::<N>().ok_or(DecodeError {
            frame_type: self.frame_type,
        })?;
        self.rest = tail;
        Ok(*head)
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        let [byte] = self.array::<1>()?;
        Ok(byte)
    }

    fn take_rest(&mut self) -> Vec<u8> {
        let rest = self.rest.to_vec();
        self.rest = &[];
        rest
    }

    /// Trailing bytes are an error: every frame has exactly one encoding.
    fn finish(self) -> Result<(), DecodeError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(DecodeError {
                frame_type: self.frame_type,
            })
        }
    }
}

/// Reassembles frames from arbitrary chunks of the byte stream.
///
/// Sans-IO: [`push`](Self::push) whatever bytes arrived, then call
/// [`next_frame`](Self::next_frame) until it returns `Ok(None)`. The buffer holds only what
/// was pushed and not yet consumed, so a driver that reads in bounded chunks and drains
/// after each push never buffers more than one maximum-size frame plus one chunk.
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    /// Bytes at the front of `buffer` that were already consumed.
    consumed: usize,
}

impl Decoder {
    /// An empty decoder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: Vec::new(),
            consumed: 0,
        }
    }

    /// Appends newly received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.consumed > 0 {
            self.buffer.drain(..self.consumed.min(self.buffer.len()));
            self.consumed = 0;
        }
        self.buffer.extend_from_slice(bytes);
    }

    /// Returns the next known frame, `Ok(None)` if more bytes are needed, or an error if the
    /// stream is corrupt. Unknown frame types are skipped. After an error the stream must be
    /// closed.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, DecodeError> {
        loop {
            let pending = self.buffer.get(self.consumed..).unwrap_or_default();
            let Some((&[frame_type, high, low], rest)) = pending.split_first_chunk::<HEADER_LEN>()
            else {
                return Ok(None);
            };
            let body_len = usize::from(u16::from_be_bytes([high, low]));
            let Some(body) = rest.get(..body_len) else {
                return Ok(None);
            };
            let parsed = Frame::parse(frame_type, body);
            self.consumed = self
                .consumed
                .saturating_add(HEADER_LEN)
                .saturating_add(body_len);
            if let Some(frame) = parsed? {
                return Ok(Some(frame));
            }
        }
    }

    /// Bytes buffered but not yet returned as a frame.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buffer.len().saturating_sub(self.consumed)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::vec;
    use alloc::vec::Vec;

    use kith_crypto::Signature;
    use kith_types::NodeId;
    use proptest::prelude::*;

    use super::{
        DecodeError, Decoder, Frame, FrameTooLarge, HealthProblem, MAX_BODY_LEN, MAX_PAYLOAD_LEN,
        RejectReason,
    };

    fn encode(frame: &Frame) -> Vec<u8> {
        let mut out = Vec::new();
        frame.encode(&mut out).unwrap();
        assert_eq!(out.len(), frame.encoded_len());
        out
    }

    fn decode_all(bytes: &[u8]) -> Result<Vec<Frame>, DecodeError> {
        let mut decoder = Decoder::new();
        decoder.push(bytes);
        let mut frames = Vec::new();
        while let Some(frame) = decoder.next_frame()? {
            frames.push(frame);
        }
        Ok(frames)
    }

    fn concat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    #[test]
    fn golden_bytes_for_every_frame_type() {
        let node = NodeId::from_bytes([0xaa; 32]);
        let cases: Vec<(Frame, Vec<u8>)> = vec![
            (
                Frame::ServerChallenge {
                    protocol_version: 1,
                    nonce: [0x11; 32],
                },
                concat(&[&[0x01, 0x00, 0x22, 0x00, 0x01], &[0x11; 32]]),
            ),
            (
                Frame::ClientHello {
                    node,
                    signature: Signature::from_bytes([0x55; 64]),
                },
                concat(&[&[0x02, 0x00, 0x60], &[0xaa; 32], &[0x55; 64]]),
            ),
            (Frame::ServerAccept, vec![0x03, 0x00, 0x00]),
            (
                Frame::ServerReject {
                    reason: RejectReason::NOT_MEMBER,
                },
                vec![0x04, 0x00, 0x01, 0x01],
            ),
            (
                Frame::SendPacket {
                    dst: node,
                    payload: vec![1, 2, 3],
                },
                concat(&[&[0x10, 0x00, 0x23], &[0xaa; 32], &[1, 2, 3]]),
            ),
            (
                Frame::RecvPacket {
                    src: node,
                    payload: vec![],
                },
                concat(&[&[0x11, 0x00, 0x20], &[0xaa; 32]]),
            ),
            (
                Frame::PeerGone { node },
                concat(&[&[0x12, 0x00, 0x20], &[0xaa; 32]]),
            ),
            (
                Frame::Ping {
                    data: [1, 2, 3, 4, 5, 6, 7, 8],
                },
                vec![0x20, 0x00, 0x08, 1, 2, 3, 4, 5, 6, 7, 8],
            ),
            (
                Frame::Pong { data: [9; 8] },
                vec![0x21, 0x00, 0x08, 9, 9, 9, 9, 9, 9, 9, 9],
            ),
            (Frame::Health { problem: None }, vec![0x22, 0x00, 0x00]),
            (
                Frame::Health {
                    problem: Some(HealthProblem::SHUTTING_DOWN),
                },
                vec![0x22, 0x00, 0x01, 0x02],
            ),
        ];
        for (frame, golden) in cases {
            assert_eq!(encode(&frame), golden, "{frame:?}");
            assert_eq!(decode_all(&golden), Ok(vec![frame]));
        }
    }

    #[test]
    fn unknown_frame_types_are_skipped_for_forward_compatibility() {
        let mut stream = vec![0x7f, 0x00, 0x02, 0xde, 0xad, 0xee, 0x00, 0x00];
        stream.extend(encode(&Frame::ServerAccept));
        assert_eq!(decode_all(&stream), Ok(vec![Frame::ServerAccept]));
    }

    #[test]
    fn known_frames_with_short_or_trailing_bytes_are_rejected() {
        // Too short, then trailing bytes.
        assert_eq!(
            decode_all(&[0x20, 0x00, 0x07, 0, 0, 0, 0, 0, 0, 0]),
            Err(DecodeError { frame_type: 0x20 })
        );
        assert_eq!(
            decode_all(&[0x03, 0x00, 0x01, 0x00]),
            Err(DecodeError { frame_type: 0x03 })
        );
        assert_eq!(
            decode_all(&[0x22, 0x00, 0x02, 0x01, 0x01]),
            Err(DecodeError { frame_type: 0x22 })
        );
        assert_eq!(
            decode_all(&[0x10, 0x00, 0x01, 0x00]),
            Err(DecodeError { frame_type: 0x10 })
        );
    }

    #[test]
    fn payload_limit_matches_length_field_and_oversize_leaves_output_untouched() {
        assert_eq!(MAX_BODY_LEN, usize::from(u16::MAX));
        assert_eq!(MAX_PAYLOAD_LEN, MAX_BODY_LEN - 32);
        let frame = Frame::SendPacket {
            dst: NodeId::from_bytes([0; 32]),
            payload: vec![0; MAX_PAYLOAD_LEN + 1],
        };
        let mut out = vec![7];
        assert_eq!(
            frame.encode(&mut out),
            Err(FrameTooLarge {
                len: MAX_BODY_LEN + 1
            })
        );
        assert_eq!(out, vec![7]);

        let largest = Frame::SendPacket {
            dst: NodeId::from_bytes([0; 32]),
            payload: vec![0; MAX_PAYLOAD_LEN],
        };
        assert_eq!(decode_all(&encode(&largest)), Ok(vec![largest]));
    }

    fn arbitrary_frame() -> impl Strategy<Value = Frame> {
        let node = any::<[u8; 32]>().prop_map(NodeId::from_bytes);
        let payload = proptest::collection::vec(any::<u8>(), 0..64);
        prop_oneof![
            (any::<u16>(), any::<[u8; 32]>()).prop_map(|(protocol_version, nonce)| {
                Frame::ServerChallenge {
                    protocol_version,
                    nonce,
                }
            }),
            (node.clone(), any::<[u8; 32]>(), any::<[u8; 32]>()).prop_map(|(node, a, b)| {
                Frame::ClientHello {
                    node,
                    signature: Signature::from_bytes([a, b].concat().try_into().unwrap()),
                }
            }),
            Just(Frame::ServerAccept),
            any::<u8>().prop_map(|code| Frame::ServerReject {
                reason: RejectReason::from_code(code)
            }),
            (node.clone(), payload.clone())
                .prop_map(|(dst, payload)| Frame::SendPacket { dst, payload }),
            (node.clone(), payload).prop_map(|(src, payload)| Frame::RecvPacket { src, payload }),
            node.prop_map(|node| Frame::PeerGone { node }),
            any::<[u8; 8]>().prop_map(|data| Frame::Ping { data }),
            any::<[u8; 8]>().prop_map(|data| Frame::Pong { data }),
            proptest::option::of(any::<u8>().prop_map(HealthProblem::from_code))
                .prop_map(|problem| Frame::Health { problem }),
        ]
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(
            chunks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..80), 0..6),
        ) {
            let mut decoder = Decoder::new();
            for chunk in &chunks {
                decoder.push(chunk);
                while let Ok(Some(_)) = decoder.next_frame() {}
            }
        }

        #[test]
        fn any_chunking_yields_the_same_frames(
            frames in proptest::collection::vec(arbitrary_frame(), 0..8),
            chunk in 1_usize..40,
        ) {
            let mut stream = Vec::new();
            for frame in &frames {
                frame.encode(&mut stream).unwrap();
            }
            let mut decoder = Decoder::new();
            let mut out = Vec::new();
            for piece in stream.chunks(chunk) {
                decoder.push(piece);
                while let Some(frame) = decoder.next_frame().unwrap() {
                    out.push(frame);
                }
            }
            prop_assert_eq!(out, frames);
            prop_assert_eq!(decoder.buffered(), 0);
        }
    }
}
