//! Access to the operating system's random source.

/// The operating system's random source failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the system random source is unavailable")]
pub struct RandomError;

/// Fills `buffer` with cryptographically secure random bytes.
#[cfg(feature = "getrandom")]
pub fn fill_random(buffer: &mut [u8]) -> Result<(), RandomError> {
    getrandom::fill(buffer).map_err(|_| RandomError)
}
