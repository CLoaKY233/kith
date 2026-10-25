//! CRC-32 (IEEE 802.3, reflected polynomial `0xEDB88320`).
//!
//! Hand-rolled with a compile-time table to keep the dependency tree small;
//! it only guards against accidental corruption, not tampering.

const POLYNOMIAL: u32 = 0xEDB8_8320;

/// Lookup table, one entry per byte value.
static TABLE: [u32; 256] = build_table();

#[expect(
    clippy::indexing_slicing,
    reason = "index runs 0..256 over a 256-entry array"
)]
const fn build_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut index = 0_usize;
    let mut byte = 0_u32;
    while index < 256 {
        let mut crc = byte;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ POLYNOMIAL
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
        byte += 1;
    }
    table
}

/// Incremental CRC-32 state.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Crc32(u32);

impl Crc32 {
    pub(crate) const fn new() -> Self {
        Self(u32::MAX)
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            let [low, ..] = (self.0 ^ u32::from(byte)).to_le_bytes();
            #[expect(
                clippy::indexing_slicing,
                reason = "a u8 index is always within the 256-entry table"
            )]
            let entry = TABLE[usize::from(low)];
            self.0 = (self.0 >> 8) ^ entry;
        }
    }

    pub(crate) const fn finish(self) -> u32 {
        !self.0
    }
}

#[cfg(test)]
mod tests {
    use super::Crc32;

    #[test]
    fn crc32_matches_standard_check_value_incrementally() {
        let mut crc = Crc32::new();
        crc.update(b"1234");
        crc.update(b"56789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
    }
}
