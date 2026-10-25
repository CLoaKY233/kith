//! Canonical unsigned LEB128 varints.
//!
//! Seven bits per byte, low bits first, high bit set on every byte except the last.
//! Only the shortest encoding is accepted, so every value has exactly one byte form.

use alloc::vec::Vec;

use crate::WireError;

/// The longest encoding of a `u64`: ceil(64 / 7) bytes.
pub const MAX_LEN: usize = 10;

/// Appends the encoding of `value` to `out`.
pub fn encode(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let low = value.to_le_bytes()[0] & 0x7f;
        value >>= 7;
        if value == 0 {
            out.push(low);
            return;
        }
        out.push(low | 0x80);
    }
}

/// Number of bytes [`encode`] would write for `value`.
#[must_use]
pub fn encoded_len(value: u64) -> usize {
    // Seven payload bits per byte; zero still takes one byte.
    let bytes = value.bit_width().div_ceil(7).max(1);
    usize::try_from(bytes).unwrap_or(MAX_LEN)
}

/// Decodes one varint from the front of `input`, returning the value and bytes consumed.
pub fn decode(input: &[u8]) -> Result<(u64, usize), WireError> {
    let mut value: u64 = 0;
    for (index, &byte) in input.iter().enumerate().take(MAX_LEN) {
        let payload = u64::from(byte & 0x7f);
        let shift = u32::try_from(index)
            .ok()
            .and_then(|i| i.checked_mul(7))
            .ok_or(WireError::BadVarint)?;
        let shifted = payload.checked_shl(shift).ok_or(WireError::BadVarint)?;
        // Bits shifted out of the top mean the value exceeds 64 bits.
        if shifted.checked_shr(shift) != Some(payload) {
            return Err(WireError::BadVarint);
        }
        value |= shifted;
        if byte & 0x80 == 0 {
            // A zero final byte (after the first) means a shorter encoding existed.
            if byte == 0 && index > 0 {
                return Err(WireError::BadVarint);
            }
            let consumed = index.checked_add(1).ok_or(WireError::BadVarint)?;
            return Ok((value, consumed));
        }
    }
    if input.len() >= MAX_LEN {
        Err(WireError::BadVarint)
    } else {
        Err(WireError::Truncated)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::{decode, encode, encoded_len};
    use crate::WireError;

    fn bytes(value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        encode(value, &mut out);
        out
    }

    #[test]
    fn known_vectors() {
        assert_eq!(bytes(0), [0x00]);
        assert_eq!(bytes(127), [0x7f]);
        assert_eq!(bytes(128), [0x80, 0x01]);
        assert_eq!(bytes(300), [0xac, 0x02]);
        assert_eq!(bytes(u64::MAX).len(), 10);
        assert_eq!(decode(&bytes(u64::MAX)), Ok((u64::MAX, 10)));
    }

    #[test]
    fn rejects_non_canonical_and_overflow() {
        assert_eq!(decode(&[0x80, 0x00]), Err(WireError::BadVarint));
        assert_eq!(decode(&[0xff; 10]), Err(WireError::BadVarint));
        // 11th byte would be needed: too long.
        assert_eq!(
            decode(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]),
            Err(WireError::BadVarint)
        );
        assert_eq!(decode(&[0x80]), Err(WireError::Truncated));
        assert_eq!(decode(&[]), Err(WireError::Truncated));
    }

    #[test]
    fn stops_at_the_first_complete_value() {
        assert_eq!(decode(&[0x05, 0xff, 0xff]), Ok((5, 1)));
    }

    proptest! {
        #[test]
        fn round_trips(value in any::<u64>()) {
            let encoded = bytes(value);
            prop_assert_eq!(encoded.len(), encoded_len(value));
            prop_assert_eq!(decode(&encoded), Ok((value, encoded.len())));
        }

        #[test]
        fn never_panics(input in proptest::collection::vec(any::<u8>(), 0..16)) {
            if let Ok((value, used)) = decode(&input) {
                prop_assert_eq!(&bytes(value)[..], &input[..used]);
            }
        }
    }
}
