//! Randomness as an input.

use crate::WgError;

/// A cryptographically secure random source, supplied by the driver.
///
/// The core never seeds or owns a generator: every ephemeral key, session index, timer
/// jitter and cookie secret comes from here. Implementations must be unpredictable to an
/// attacker; a deterministic source is only acceptable in tests.
pub trait RandomSource {
    /// Fills `buffer` entirely with random bytes.
    fn fill_bytes(&mut self, buffer: &mut [u8]) -> Result<(), WgError>;
}

/// Draws a random `u32`.
pub(crate) fn random_u32(source: &mut dyn RandomSource) -> Result<u32, WgError> {
    let mut bytes = [0_u8; 4];
    source.fill_bytes(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
