//! CRC-32 (IEEE 802.3, reflected polynomial `0xEDB88320`), as FINGERPRINT requires.
//!
//! Hand-rolled with a compile-time table: it is twenty lines, and a dependency
//! for it would be a larger audit surface than the code itself.

/// Reflected CRC-32 polynomial.
const POLYNOMIAL: u32 = 0xEDB8_8320;

/// Byte-at-a-time lookup table, built at compile time.
const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut byte: u32 = 0;
    while byte < 256 {
        let mut crc = byte;
        let mut bit: u32 = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ POLYNOMIAL
            } else {
                crc >> 1
            };
            bit = bit.wrapping_add(1);
        }
        #[expect(
            clippy::indexing_slicing,
            clippy::as_conversions,
            reason = "const fn: `byte < 256` is the loop condition, and u32 to usize is lossless"
        )]
        {
            table[byte as usize] = crc;
        }
        byte = byte.wrapping_add(1);
    }
    table
}

/// CRC-32 of `data`.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let crc = data.iter().fold(u32::MAX, |crc, &byte| {
        let index = usize::from(crc.to_le_bytes()[0] ^ byte);
        TABLE.get(index).copied().unwrap_or(0) ^ (crc >> 8)
    });
    !crc
}

#[cfg(test)]
mod tests {
    use super::crc32;

    #[test]
    fn check_value() {
        // The standard CRC-32 check value.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }
}
