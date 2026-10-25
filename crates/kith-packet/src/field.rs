//! Bounds-checked big-endian field access.
//!
//! Views validate lengths up front, so these helpers never hit their fallback
//! in practice. They exist so that a validation bug yields a wrong value
//! instead of a panic on the hot path.

/// Reads `N` bytes at `offset`, or zeros if out of range.
pub(crate) fn bytes<const N: usize>(buffer: &[u8], offset: usize) -> [u8; N] {
    buffer
        .get(offset..)
        .and_then(<[u8]>::first_chunk::<N>)
        .copied()
        .unwrap_or([0; N])
}

/// Reads one byte at `offset`, or zero if out of range.
pub(crate) fn u8_at(buffer: &[u8], offset: usize) -> u8 {
    buffer.get(offset).copied().unwrap_or(0)
}

/// Reads a big-endian `u16` at `offset`, or zero if out of range.
pub(crate) fn u16_at(buffer: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes(buffer, offset))
}

/// Reads a big-endian `u32` at `offset`, or zero if out of range.
pub(crate) fn u32_at(buffer: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes(buffer, offset))
}

/// Writes `value` at `offset`; does nothing if out of range.
pub(crate) fn set_bytes<const N: usize>(buffer: &mut [u8], offset: usize, value: [u8; N]) {
    if let Some(slot) = buffer
        .get_mut(offset..)
        .and_then(<[u8]>::first_chunk_mut::<N>)
    {
        *slot = value;
    }
}

/// Writes a big-endian `u16` at `offset`; does nothing if out of range.
pub(crate) fn set_u16(buffer: &mut [u8], offset: usize, value: u16) {
    set_bytes(buffer, offset, value.to_be_bytes());
}

/// Returns `buffer[start..end]`, or an empty slice if out of range.
pub(crate) fn slice(buffer: &[u8], start: usize, end: usize) -> &[u8] {
    buffer.get(start..end).unwrap_or(&[])
}
