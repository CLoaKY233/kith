//! RFC 6455 §5 framing, sans-IO. Only binary data messages are accepted.

use crate::Error;

pub(crate) const OP_CONTINUATION: u8 = 0x0;
pub(crate) const OP_BINARY: u8 = 0x2;
pub(crate) const OP_CLOSE: u8 = 0x8;
pub(crate) const OP_PING: u8 = 0x9;
pub(crate) const OP_PONG: u8 = 0xA;
const MAX_CONTROL_PAYLOAD: u64 = 125;

/// One decoded event.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    Close,
}

fn apply_mask(payload: &mut [u8], mask: [u8; 4]) {
    for (byte, key) in payload.iter_mut().zip(mask.iter().cycle()) {
        *byte ^= key;
    }
}

/// Appends one frame. Clients must pass a mask (§5.3), servers must not.
pub(crate) fn encode(
    out: &mut Vec<u8>,
    fin: bool,
    opcode: u8,
    payload: &[u8],
    mask: Option<[u8; 4]>,
) {
    out.push(if fin { 0x80 } else { 0 } | (opcode & 0x0F));
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    let length = payload.len();
    if let Ok(short) = u8::try_from(length)
        && short < 126
    {
        out.push(mask_bit | short);
    } else if let Ok(medium) = u16::try_from(length) {
        out.push(mask_bit | 0x7E);
        out.extend_from_slice(&medium.to_be_bytes());
    } else {
        out.push(mask_bit | 0x7F);
        out.extend_from_slice(&u64::try_from(length).unwrap_or(u64::MAX).to_be_bytes());
    }
    let Some(key) = mask else {
        out.extend_from_slice(payload);
        return;
    };
    out.extend_from_slice(&key);
    let start = out.len();
    out.extend_from_slice(payload);
    if let Some(masked) = out.get_mut(start..) {
        apply_mask(masked, key);
    }
}

/// Incremental decoder with fragment reassembly bounded by `max_message`.
#[derive(Debug)]
pub(crate) struct Decoder {
    buffer: Vec<u8>,
    message: Vec<u8>,
    in_message: bool,
    expect_masked: bool,
    max_message: usize,
}

impl Decoder {
    /// `expect_masked` is true on the server side.
    pub(crate) fn new(expect_masked: bool, max_message: usize) -> Self {
        Self {
            buffer: Vec::new(),
            message: Vec::new(),
            in_message: false,
            expect_masked,
            max_message,
        }
    }

    pub(crate) fn push(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// The next complete event, or `None` if more bytes are needed. Errors are fatal.
    pub(crate) fn next_event(&mut self) -> Result<Option<Event>, Error> {
        loop {
            let Some((fin, opcode, payload)) = self.next_frame()? else {
                return Ok(None);
            };
            match opcode {
                OP_BINARY | OP_CONTINUATION => {
                    if (opcode == OP_BINARY) == self.in_message {
                        return Err(Error::Protocol("unexpected data frame"));
                    }
                    if self.message.len() + payload.len() > self.max_message {
                        return Err(Error::TooLarge);
                    }
                    self.message.extend_from_slice(&payload);
                    self.in_message = !fin;
                    if fin {
                        return Ok(Some(Event::Binary(core::mem::take(&mut self.message))));
                    }
                }
                OP_PING => return Ok(Some(Event::Ping(payload))),
                OP_PONG => return Ok(Some(Event::Pong)),
                OP_CLOSE if payload.len() != 1 => return Ok(Some(Event::Close)),
                _ => return Err(Error::Protocol("bad opcode or close payload")),
            }
        }
    }

    fn next_frame(&mut self) -> Result<Option<(bool, u8, Vec<u8>)>, Error> {
        let (Some(&b0), Some(&b1)) = (self.buffer.first(), self.buffer.get(1)) else {
            return Ok(None);
        };
        if b0 & 0x70 != 0 {
            return Err(Error::Protocol("reserved bits set"));
        }
        let fin = b0 & 0x80 != 0;
        let opcode = b0 & 0x0F;
        if (b1 & 0x80 != 0) != self.expect_masked {
            return Err(Error::Protocol("wrong masking"));
        }
        let (length, mut offset) = match b1 & 0x7F {
            126 => {
                let Some(&[a, b]) = self.buffer.get(2..4) else {
                    return Ok(None);
                };
                let length = u16::from_be_bytes([a, b]);
                if length < 126 {
                    return Err(Error::Protocol("non-minimal length"));
                }
                (u64::from(length), 4)
            }
            127 => {
                let Some(bytes) = self.buffer.get(2..10) else {
                    return Ok(None);
                };
                let mut raw = [0u8; 8];
                raw.copy_from_slice(bytes);
                let length = u64::from_be_bytes(raw);
                if u16::try_from(length).is_ok() {
                    return Err(Error::Protocol("non-minimal length"));
                }
                (length, 10)
            }
            short => (u64::from(short), 2),
        };
        if opcode >= OP_CLOSE && (!fin || length > MAX_CONTROL_PAYLOAD) {
            return Err(Error::Protocol("bad control frame"));
        }
        let length = usize::try_from(length).map_err(|_| Error::TooLarge)?;
        if length > self.max_message {
            return Err(Error::TooLarge);
        }
        let mask = if self.expect_masked {
            let Some(&[a, b, c, d]) = self.buffer.get(offset..offset + 4) else {
                return Ok(None);
            };
            offset += 4;
            Some([a, b, c, d])
        } else {
            None
        };
        let Some(payload) = self.buffer.get(offset..offset + length) else {
            return Ok(None);
        };
        let mut payload = payload.to_vec();
        if let Some(key) = mask {
            apply_mask(&mut payload, key);
        }
        self.buffer.drain(..offset + length);
        Ok(Some((fin, opcode, payload)))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const LIMIT: usize = 1 << 17;

    #[test]
    fn frames_match_rfc_6455_section_5_7() {
        let mut out = Vec::new();
        encode(
            &mut out,
            true,
            0x1,
            b"Hello",
            Some([0x37, 0xfa, 0x21, 0x3d]),
        );
        assert_eq!(
            out,
            [
                0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58
            ]
        );

        let mut server = Decoder::new(true, LIMIT);
        server.push(&[
            0x8a, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
        ]);
        assert_eq!(server.next_event().unwrap(), Some(Event::Pong));

        let mut client = Decoder::new(false, LIMIT);
        client.push(&[0x89, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f]);
        assert_eq!(
            client.next_event().unwrap(),
            Some(Event::Ping(b"Hello".to_vec()))
        );

        for (len, header) in [
            (256, vec![0x82, 0x7E, 0x01, 0x00]),
            (65536, vec![0x82, 0x7F, 0, 0, 0, 0, 0, 1, 0, 0]),
        ] {
            let mut out = Vec::new();
            encode(&mut out, true, OP_BINARY, &vec![7; len], None);
            assert_eq!(out[..header.len()], header[..]);
            client.push(&out);
            assert_eq!(
                client.next_event().unwrap(),
                Some(Event::Binary(vec![7; len]))
            );
        }
    }

    proptest! {
        #[test]
        fn fragmented_messages_reassemble_from_any_chunking(
            messages in proptest::collection::vec(
                (proptest::collection::vec(any::<u8>(), 0..70_000), 1usize..4, any::<bool>()), 1..6),
            masked in any::<bool>(),
            chunk in 1usize..5000,
        ) {
            let mut wire = Vec::new();
            for (message, pieces, ping) in &messages {
                let size = message.len().div_ceil(*pieces).max(1);
                let parts: Vec<&[u8]> = if message.is_empty() { vec![&[]] } else { message.chunks(size).collect() };
                for (i, part) in parts.iter().enumerate() {
                    let opcode = if i == 0 { OP_BINARY } else { OP_CONTINUATION };
                    encode(&mut wire, i + 1 == parts.len(), opcode, part, masked.then_some([1, 2, 3, 4]));
                    if *ping {
                        encode(&mut wire, true, OP_PING, b"p", masked.then_some([9, 8, 7, 6]));
                    }
                }
            }
            let mut decoder = Decoder::new(masked, LIMIT);
            let mut received = Vec::new();
            for piece in wire.chunks(chunk) {
                decoder.push(piece);
                while let Some(event) = decoder.next_event().unwrap() {
                    if let Event::Binary(data) = event {
                        received.push(data);
                    }
                }
            }
            let sent: Vec<Vec<u8>> = messages.into_iter().map(|(m, _, _)| m).collect();
            prop_assert_eq!(received, sent);
        }

        #[test]
        fn decoder_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096), masked in any::<bool>()) {
            let mut decoder = Decoder::new(masked, 1024);
            decoder.push(&bytes);
            while let Ok(Some(_)) = decoder.next_event() {}
        }
    }
}
