//! BLAKE3 hashing and key derivation, always domain-separated.

use core::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::Context;

/// A 32-byte BLAKE3 digest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hash([u8; 32]);

impl Hash {
    /// Hashes `data` under `context`.
    #[must_use]
    pub fn of(context: Context, data: &[u8]) -> Self {
        let mut hasher = Hasher::new(context);
        hasher.update(data);
        hasher.finalize()
    }

    /// Wraps a digest that was computed elsewhere.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrows the digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Copies out the digest.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash(")?;
        for byte in self.0.iter().take(4) {
            write!(f, "{byte:02x}")?;
        }
        write!(f, ")")
    }
}

/// Incremental hashing under a context.
#[derive(Clone)]
pub struct Hasher(blake3::Hasher);

impl Hasher {
    /// Starts a hash bound to `context`.
    #[must_use]
    pub fn new(context: Context) -> Self {
        Self(blake3::Hasher::new_derive_key(context.as_str()))
    }

    /// Feeds bytes into the hash.
    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.0.update(data);
        self
    }

    /// Finishes the hash.
    #[must_use]
    pub fn finalize(&self) -> Hash {
        Hash(*self.0.finalize().as_bytes())
    }
}

impl fmt::Debug for Hasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Hasher")
    }
}

/// Derives a 32-byte secret from `material` under `context`. The output zeroizes on drop.
#[must_use]
pub fn derive_key(context: Context, material: &[u8]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(blake3::derive_key(context.as_str(), material))
}

#[cfg(test)]
mod tests {
    use super::{Context, Hash, Hasher, derive_key};

    const A: Context = Context::new("kith test hash a");
    const B: Context = Context::new("kith test hash b");

    #[test]
    fn contexts_separate_domains() {
        assert_ne!(Hash::of(A, b"same input"), Hash::of(B, b"same input"));
        assert_ne!(*derive_key(A, b"material"), *derive_key(B, b"material"));
    }

    #[test]
    fn incremental_matches_one_shot() {
        let mut hasher = Hasher::new(A);
        hasher.update(b"split ").update(b"input");
        assert_eq!(hasher.finalize(), Hash::of(A, b"split input"));
    }

    #[test]
    fn matches_blake3_derive_key_mode() {
        // Golden value: pins the construction so it can never change silently.
        assert_eq!(
            Hash::of(A, b"").to_bytes(),
            blake3::derive_key("kith test hash a", b"")
        );
    }
}
