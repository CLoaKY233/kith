//! Wire codecs for the four message types (whitepaper §5.4).

#![warn(clippy::arithmetic_side_effects)]

use crate::WgError;

/// Length of a handshake initiation.
pub(crate) const HANDSHAKE_INITIATION_LEN: usize = 148;
/// Length of a handshake response.
pub(crate) const HANDSHAKE_RESPONSE_LEN: usize = 92;
/// Length of a cookie reply.
pub(crate) const COOKIE_REPLY_LEN: usize = 64;
/// Length of the transport data header (type, reserved, receiver, counter).
pub(crate) const TRANSPORT_HEADER_LEN: usize = 16;
/// Header plus authentication tag of a transport message.
pub(crate) const TRANSPORT_OVERHEAD: usize = TRANSPORT_HEADER_LEN + kith_crypto::TAG_LEN;
/// Worst-case bytes a transport message adds to a plaintext, including padding to 16.
pub const MAX_OUTPUT_OVERHEAD: usize = TRANSPORT_OVERHEAD + 15;
/// Every output buffer must be at least this long, so any handshake message fits.
pub const MIN_OUTPUT_LEN: usize = HANDSHAKE_INITIATION_LEN;

const TYPE_INITIATION: u8 = 1;
const TYPE_RESPONSE: u8 = 2;
const TYPE_COOKIE_REPLY: u8 = 3;
const TYPE_TRANSPORT: u8 = 4;

/// MAC1 covers the message up to MAC1; MAC2 covers it up to MAC2.
pub(crate) const INITIATION_MAC1_OFFSET: usize = 116;
pub(crate) const RESPONSE_MAC1_OFFSET: usize = 60;
pub(crate) const MAC_LEN: usize = 16;

/// The output buffer length needed to encapsulate a plaintext of `plaintext_len` bytes.
#[must_use]
pub const fn required_output_len(plaintext_len: usize) -> usize {
    let transport = plaintext_len.saturating_add(MAX_OUTPUT_OVERHEAD);
    if transport > MIN_OUTPUT_LEN {
        transport
    } else {
        MIN_OUTPUT_LEN
    }
}

/// Zero-padded plaintext length: the next multiple of 16 (§5.4.6).
pub(crate) const fn padded_len(plaintext_len: usize) -> usize {
    plaintext_len.saturating_add(15) & !15
}

/// Sequential reader that fails instead of indexing out of bounds.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], WgError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(WgError::MalformedMessage)?;
        self.0 = rest;
        Ok(*head)
    }

    fn u32(&mut self) -> Result<u32, WgError> {
        self.take().map(u32::from_le_bytes)
    }

    fn rest(self) -> &'a [u8] {
        self.0
    }
}

/// Sequential writer into a caller buffer.
struct Writer<'a> {
    buffer: &'a mut [u8],
    written: usize,
}

impl<'a> Writer<'a> {
    fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, written: 0 }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), WgError> {
        let end = self
            .written
            .checked_add(bytes.len())
            .ok_or(WgError::BufferTooSmall)?;
        self.buffer
            .get_mut(self.written..end)
            .ok_or(WgError::BufferTooSmall)?
            .copy_from_slice(bytes);
        self.written = end;
        Ok(())
    }

    fn header(&mut self, message_type: u8) -> Result<(), WgError> {
        self.put(&[message_type, 0, 0, 0])
    }
}

/// A handshake initiation (§5.4.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Initiation {
    pub(crate) sender: u32,
    pub(crate) ephemeral: [u8; 32],
    pub(crate) encrypted_static: [u8; 48],
    pub(crate) encrypted_timestamp: [u8; 28],
    pub(crate) mac1: [u8; MAC_LEN],
    pub(crate) mac2: [u8; MAC_LEN],
}

impl Initiation {
    pub(crate) fn to_bytes(&self) -> [u8; HANDSHAKE_INITIATION_LEN] {
        let mut bytes = [0; HANDSHAKE_INITIATION_LEN];
        let mut writer = Writer::new(&mut bytes);
        let written = writer
            .header(TYPE_INITIATION)
            .and_then(|()| writer.put(&self.sender.to_le_bytes()))
            .and_then(|()| writer.put(&self.ephemeral))
            .and_then(|()| writer.put(&self.encrypted_static))
            .and_then(|()| writer.put(&self.encrypted_timestamp))
            .and_then(|()| writer.put(&self.mac1))
            .and_then(|()| writer.put(&self.mac2));
        debug_assert!(written.is_ok(), "fields sum to the fixed message length");
        bytes
    }
}

/// A handshake response (§5.4.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Response {
    pub(crate) sender: u32,
    pub(crate) receiver: u32,
    pub(crate) ephemeral: [u8; 32],
    pub(crate) encrypted_nothing: [u8; 16],
    pub(crate) mac1: [u8; MAC_LEN],
    pub(crate) mac2: [u8; MAC_LEN],
}

impl Response {
    pub(crate) fn to_bytes(&self) -> [u8; HANDSHAKE_RESPONSE_LEN] {
        let mut bytes = [0; HANDSHAKE_RESPONSE_LEN];
        let mut writer = Writer::new(&mut bytes);
        let written = writer
            .header(TYPE_RESPONSE)
            .and_then(|()| writer.put(&self.sender.to_le_bytes()))
            .and_then(|()| writer.put(&self.receiver.to_le_bytes()))
            .and_then(|()| writer.put(&self.ephemeral))
            .and_then(|()| writer.put(&self.encrypted_nothing))
            .and_then(|()| writer.put(&self.mac1))
            .and_then(|()| writer.put(&self.mac2));
        debug_assert!(written.is_ok(), "fields sum to the fixed message length");
        bytes
    }
}

/// A cookie reply (§5.4.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CookieReply {
    pub(crate) receiver: u32,
    pub(crate) nonce: [u8; 24],
    pub(crate) encrypted_cookie: [u8; 32],
}

impl CookieReply {
    pub(crate) fn to_bytes(&self) -> [u8; COOKIE_REPLY_LEN] {
        let mut bytes = [0; COOKIE_REPLY_LEN];
        let mut writer = Writer::new(&mut bytes);
        let written = writer
            .header(TYPE_COOKIE_REPLY)
            .and_then(|()| writer.put(&self.receiver.to_le_bytes()))
            .and_then(|()| writer.put(&self.nonce))
            .and_then(|()| writer.put(&self.encrypted_cookie));
        debug_assert!(written.is_ok(), "fields sum to the fixed message length");
        bytes
    }
}

/// A transport data message, borrowing its ciphertext (§5.4.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Transport<'a> {
    pub(crate) receiver: u32,
    pub(crate) counter: u64,
    /// Encrypted, padded payload followed by the 16-byte tag.
    pub(crate) sealed: &'a [u8],
}

/// Writes a transport header into the front of `out`.
pub(crate) fn write_transport_header(
    out: &mut [u8],
    receiver: u32,
    counter: u64,
) -> Result<(), WgError> {
    let mut writer = Writer::new(out);
    writer.header(TYPE_TRANSPORT)?;
    writer.put(&receiver.to_le_bytes())?;
    writer.put(&counter.to_le_bytes())
}

/// Any parsed message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Message<'a> {
    Initiation(Initiation),
    Response(Response),
    CookieReply(CookieReply),
    Transport(Transport<'a>),
}

impl<'a> Message<'a> {
    /// Parses a datagram, demanding the exact length for its type and zero reserved bytes.
    pub(crate) fn parse(datagram: &'a [u8]) -> Result<Self, WgError> {
        let mut reader = Reader(datagram);
        let [message_type, 0, 0, 0] = reader.take::<4>()? else {
            return Err(WgError::MalformedMessage);
        };
        let exact = |expected: usize| {
            if datagram.len() == expected {
                Ok(())
            } else {
                Err(WgError::MalformedMessage)
            }
        };
        match message_type {
            TYPE_INITIATION => {
                exact(HANDSHAKE_INITIATION_LEN)?;
                Ok(Self::Initiation(Initiation {
                    sender: reader.u32()?,
                    ephemeral: reader.take()?,
                    encrypted_static: reader.take()?,
                    encrypted_timestamp: reader.take()?,
                    mac1: reader.take()?,
                    mac2: reader.take()?,
                }))
            }
            TYPE_RESPONSE => {
                exact(HANDSHAKE_RESPONSE_LEN)?;
                Ok(Self::Response(Response {
                    sender: reader.u32()?,
                    receiver: reader.u32()?,
                    ephemeral: reader.take()?,
                    encrypted_nothing: reader.take()?,
                    mac1: reader.take()?,
                    mac2: reader.take()?,
                }))
            }
            TYPE_COOKIE_REPLY => {
                exact(COOKIE_REPLY_LEN)?;
                Ok(Self::CookieReply(CookieReply {
                    receiver: reader.u32()?,
                    nonce: reader.take()?,
                    encrypted_cookie: reader.take()?,
                }))
            }
            TYPE_TRANSPORT => {
                let receiver = reader.u32()?;
                let counter = u64::from_le_bytes(reader.take()?);
                let sealed = reader.rest();
                if sealed.len() < kith_crypto::TAG_LEN {
                    return Err(WgError::MalformedMessage);
                }
                Ok(Self::Transport(Transport {
                    receiver,
                    counter,
                    sealed,
                }))
            }
            _ => Err(WgError::MalformedMessage),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use proptest::prelude::*;

    use super::{CookieReply, Initiation, Message, Response, Transport, write_transport_header};

    fn pattern<const N: usize>(seed: u8) -> [u8; N] {
        core::array::from_fn(|i| seed.wrapping_add(u8::try_from(i % 256).unwrap()))
    }

    /// Pins the byte layout of every message: a change here is a protocol break.
    #[test]
    fn messages_have_golden_layouts() {
        let initiation = Initiation {
            sender: 0x0403_0201,
            ephemeral: [0xe0; 32],
            encrypted_static: [0x5a; 48],
            encrypted_timestamp: [0x7e; 28],
            mac1: [0x11; 16],
            mac2: [0x22; 16],
        };
        let mut expected = std::vec![1, 0, 0, 0, 1, 2, 3, 4];
        expected.extend([0xe0; 32]);
        expected.extend([0x5a; 48]);
        expected.extend([0x7e; 28]);
        expected.extend([0x11; 16]);
        expected.extend([0x22; 16]);
        assert_eq!(initiation.to_bytes().as_slice(), expected.as_slice());
        assert_eq!(
            Message::parse(&expected).unwrap(),
            Message::Initiation(initiation)
        );

        let response = Response {
            sender: 0x0807_0605,
            receiver: 0x0403_0201,
            ephemeral: [0xe1; 32],
            encrypted_nothing: [0x33; 16],
            mac1: [0x44; 16],
            mac2: [0x55; 16],
        };
        let mut expected = std::vec![2, 0, 0, 0, 5, 6, 7, 8, 1, 2, 3, 4];
        expected.extend([0xe1; 32]);
        expected.extend([0x33; 16]);
        expected.extend([0x44; 16]);
        expected.extend([0x55; 16]);
        assert_eq!(response.to_bytes().as_slice(), expected.as_slice());
        assert_eq!(
            Message::parse(&expected).unwrap(),
            Message::Response(response)
        );

        let reply = CookieReply {
            receiver: 0x0403_0201,
            nonce: pattern(0x90),
            encrypted_cookie: pattern(0xc0),
        };
        let mut expected = std::vec![3, 0, 0, 0, 1, 2, 3, 4];
        expected.extend(pattern::<24>(0x90));
        expected.extend(pattern::<32>(0xc0));
        assert_eq!(reply.to_bytes().as_slice(), expected.as_slice());
        assert_eq!(
            Message::parse(&expected).unwrap(),
            Message::CookieReply(reply)
        );

        let mut transport = [0xaa_u8; 32];
        write_transport_header(&mut transport, 0x0403_0201, 0x0807_0605_0403_0201).unwrap();
        assert_eq!(
            transport[..16],
            [4, 0, 0, 0, 1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(
            Message::parse(&transport).unwrap(),
            Message::Transport(Transport {
                receiver: 0x0403_0201,
                counter: 0x0807_0605_0403_0201,
                sealed: &[0xaa; 16],
            })
        );
    }

    fn datagram() -> impl Strategy<Value = std::vec::Vec<u8>> {
        (
            0_u8..6,
            prop::bool::weighted(0.9),
            prop_oneof![Just(148_usize), Just(92), Just(64), 0_usize..200],
            any::<u64>(),
        )
            .prop_map(|(kind, zero_reserved, len, seed)| {
                let mut bytes: std::vec::Vec<u8> = seed
                    .to_le_bytes()
                    .into_iter()
                    .cycle()
                    .zip((0_u8..=255).cycle())
                    .map(|(byte, i)| byte ^ i)
                    .take(len)
                    .collect();
                if let Some(first) = bytes.first_mut() {
                    *first = kind;
                }
                if zero_reserved {
                    bytes.iter_mut().skip(1).take(3).for_each(|byte| *byte = 0);
                }
                bytes
            })
    }

    proptest! {
        /// Accepted handshake messages re-encode to exactly the input, so MACs computed over
        /// the re-encoding cover the bytes received; anything else is rejected, never a panic.
        #[test]
        fn parse_is_strict_and_total(datagram in datagram()) {
            let reencoded = match Message::parse(&datagram) {
                Ok(Message::Initiation(m)) => m.to_bytes().to_vec(),
                Ok(Message::Response(m)) => m.to_bytes().to_vec(),
                Ok(Message::CookieReply(m)) => m.to_bytes().to_vec(),
                Ok(Message::Transport(m)) => {
                    prop_assert_eq!(&datagram[..4], &[4, 0, 0, 0]);
                    prop_assert!(m.sealed.len() >= 16);
                    prop_assert_eq!(m.sealed, &datagram[16..]);
                    return Ok(());
                }
                Err(_) => return Ok(()),
            };
            prop_assert_eq!(reencoded, datagram);
        }
    }
}
