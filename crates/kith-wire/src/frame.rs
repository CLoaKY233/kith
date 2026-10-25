//! Length-prefixed framing for byte streams (TCP, QUIC streams, Unix sockets).
//!
//! ```text
//! length (varint) | payload (length bytes)
//! ```
//!
//! [`FrameDecoder`] is sans-IO: feed it whatever bytes arrived, then pull complete frames.

use alloc::vec::Vec;

use crate::{WireError, varint};

/// Appends one frame carrying `payload` to `out`.
pub fn encode(payload: &[u8], out: &mut Vec<u8>) {
    let len = u64::try_from(payload.len()).unwrap_or(u64::MAX);
    out.reserve(varint::encoded_len(len).saturating_add(payload.len()));
    varint::encode(len, out);
    out.extend_from_slice(payload);
}

/// Reassembles frames from arbitrary chunks of a byte stream.
#[derive(Debug)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    /// Bytes at the front of `buffer` that were already returned.
    consumed: usize,
    max_frame_len: usize,
}

impl FrameDecoder {
    /// A decoder that rejects frames longer than `max_frame_len`.
    #[must_use]
    pub const fn new(max_frame_len: usize) -> Self {
        Self {
            buffer: Vec::new(),
            consumed: 0,
            max_frame_len,
        }
    }

    /// Appends newly received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        self.compact();
        self.buffer.extend_from_slice(bytes);
    }

    /// Returns the next complete frame, `Ok(None)` if more bytes are needed, or an error if
    /// the stream is corrupt. After an error the stream must be closed.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, WireError> {
        let pending = self.buffer.get(self.consumed..).unwrap_or_default();
        let (len, prefix) = match varint::decode(pending) {
            Ok(decoded) => decoded,
            Err(WireError::Truncated) => return Ok(None),
            Err(error) => return Err(error),
        };
        let max = u64::try_from(self.max_frame_len).unwrap_or(u64::MAX);
        if len > max {
            return Err(WireError::TooLarge { len, max });
        }
        let len = usize::try_from(len).map_err(|_| WireError::TooLarge { len, max })?;
        let end = prefix
            .checked_add(len)
            .ok_or(WireError::TooLarge { len: max, max })?;
        let Some(payload) = pending.get(prefix..end) else {
            return Ok(None);
        };
        let frame = payload.to_vec();
        self.consumed = self.consumed.saturating_add(end);
        Ok(Some(frame))
    }

    /// Bytes buffered but not yet returned as a frame.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buffer.len().saturating_sub(self.consumed)
    }

    /// Drops already-returned bytes so the buffer does not grow without bound.
    fn compact(&mut self) {
        if self.consumed > 0 {
            self.buffer.drain(..self.consumed.min(self.buffer.len()));
            self.consumed = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::vec;
    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::{FrameDecoder, encode};
    use crate::WireError;

    #[test]
    fn reassembles_split_and_coalesced_frames() {
        let mut stream = Vec::new();
        encode(b"one", &mut stream);
        encode(b"", &mut stream);
        encode(b"three", &mut stream);

        let mut decoder = FrameDecoder::new(64);
        let (head, tail) = stream.split_at(2);
        decoder.push(head);
        assert_eq!(decoder.next_frame(), Ok(None));
        decoder.push(tail);
        assert_eq!(decoder.next_frame(), Ok(Some(b"one".to_vec())));
        assert_eq!(decoder.next_frame(), Ok(Some(vec![])));
        assert_eq!(decoder.next_frame(), Ok(Some(b"three".to_vec())));
        assert_eq!(decoder.next_frame(), Ok(None));
        assert_eq!(decoder.buffered(), 0);
    }

    #[test]
    fn rejects_oversized_frames_before_buffering_them() {
        let mut stream = Vec::new();
        encode(&[0; 100], &mut stream);
        let mut decoder = FrameDecoder::new(10);
        decoder.push(&stream[..2]);
        assert_eq!(
            decoder.next_frame(),
            Err(WireError::TooLarge { len: 100, max: 10 })
        );
    }

    proptest! {
        #[test]
        fn any_chunking_yields_the_same_frames(
            frames in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..40), 0..8),
            chunk in 1_usize..17,
        ) {
            let mut stream = Vec::new();
            for frame in &frames {
                encode(frame, &mut stream);
            }
            let mut decoder = FrameDecoder::new(64);
            let mut out = Vec::new();
            for piece in stream.chunks(chunk) {
                decoder.push(piece);
                while let Some(frame) = decoder.next_frame().unwrap() {
                    out.push(frame);
                }
            }
            prop_assert_eq!(out, frames);
        }

        #[test]
        fn garbage_never_panics(input in proptest::collection::vec(any::<u8>(), 0..64)) {
            let mut decoder = FrameDecoder::new(32);
            decoder.push(&input);
            while let Ok(Some(_)) = decoder.next_frame() {}
        }
    }
}
