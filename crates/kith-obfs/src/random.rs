/// A source of random bytes supplied by the caller.
pub trait RandomSource {
    /// Fills `buf` entirely with random bytes.
    fn fill(&mut self, buf: &mut [u8]);
}

impl<R: RandomSource + ?Sized> RandomSource for &mut R {
    fn fill(&mut self, buf: &mut [u8]) {
        (**self).fill(buf);
    }
}

/// Draws a `u64` from `rng`.
pub(crate) fn next_u64(rng: &mut impl RandomSource) -> u64 {
    let mut bytes = [0_u8; 8];
    rng.fill(&mut bytes);
    u64::from_le_bytes(bytes)
}

/// Draws a value uniformly (up to negligible modulo bias) from `low..=high`; `low` if empty.
pub(crate) fn uniform_u32(rng: &mut impl RandomSource, low: u32, high: u32) -> u32 {
    let Some(width) = high.checked_sub(low) else {
        return low;
    };
    // `width + 1` is at most 2^32, so it fits in a u64 and is never zero.
    let span = u64::from(width).wrapping_add(1);
    let offset = next_u64(rng).checked_rem(span).unwrap_or(0);
    // `offset < span <= 2^32`, so the conversion succeeds and `low + offset <= high`.
    u32::try_from(offset).map_or(low, |offset| low.saturating_add(offset))
}

/// Like [`uniform_u32`], for `u16` bounds.
pub(crate) fn uniform_u16(rng: &mut impl RandomSource, low: u16, high: u16) -> u16 {
    let value = uniform_u32(rng, u32::from(low), u32::from(high));
    u16::try_from(value).unwrap_or(low)
}
