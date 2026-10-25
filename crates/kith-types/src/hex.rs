//! Lowercase hexadecimal encoding without allocation.

use core::fmt;

const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Writes `bytes` as lowercase hex.
pub(crate) fn write(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        let high = DIGITS.get(usize::from(byte >> 4)).copied().unwrap_or(b'?');
        let low = DIGITS
            .get(usize::from(byte & 0x0f))
            .copied()
            .unwrap_or(b'?');
        write!(f, "{}{}", char::from(high), char::from(low))?;
    }
    Ok(())
}

/// Value of one hex digit (either case), or `None`.
const fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// Decodes exactly `N` bytes from `text`. Returns `None` on wrong length or a bad digit.
pub(crate) fn decode<const N: usize>(text: &str) -> Option<[u8; N]> {
    let digits = text.as_bytes();
    if digits.len() != N.checked_mul(2)? {
        return None;
    }
    let (pairs, _remainder) = digits.as_chunks::<2>();
    let mut out = [0_u8; N];
    for (slot, [high, low]) in out.iter_mut().zip(pairs) {
        *slot = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_both_cases() {
        assert_eq!(decode::<2>("0aFf"), Some([0x0a, 0xff]));
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(decode::<2>("0aF"), None);
        assert_eq!(decode::<2>("0aFg"), None);
        assert_eq!(decode::<2>(""), None);
    }
}
