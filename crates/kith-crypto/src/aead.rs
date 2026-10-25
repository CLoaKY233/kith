//! ChaCha20-Poly1305 authenticated encryption.
//!
//! Chosen over AES-GCM because it is fast in pure software, which matters on routers without
//! AES instructions.

use alloc::vec::Vec;
use core::fmt;

use chacha20poly1305::ChaCha20Poly1305;
use chacha20poly1305::aead::{AeadInOut as _, KeyInit as _};

/// Length of an authentication tag in bytes.
pub const TAG_LEN: usize = 16;

/// Encryption or decryption failed. Deliberately carries no detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("authenticated encryption failed")]
pub struct AeadError;

/// A 96-bit nonce. Never reuse a nonce with the same key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Nonce([u8; 12]);

impl Nonce {
    /// Wraps raw nonce bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 12]) -> Self {
        Self(bytes)
    }

    /// A nonce from a 64-bit counter (little-endian, zero-padded), as WireGuard does.
    #[must_use]
    pub fn from_counter(counter: u64) -> Self {
        let mut bytes = [0_u8; 12];
        for (slot, byte) in bytes.iter_mut().skip(4).zip(counter.to_le_bytes()) {
            *slot = byte;
        }
        Self(bytes)
    }
}

/// A detached authentication tag.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tag([u8; TAG_LEN]);

impl Tag {
    /// Wraps raw tag bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; TAG_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw tag bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; TAG_LEN] {
        self.0
    }
}

/// A symmetric key. Zeroizes on drop and is not `Clone`.
pub struct AeadKey(ChaCha20Poly1305);

impl AeadKey {
    /// Builds a key from 32 secret bytes (usually from [`crate::derive_key`]).
    #[must_use]
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(ChaCha20Poly1305::new(&(*bytes).into()))
    }

    /// Encrypts `buffer` in place and returns the tag.
    pub fn seal_in_place(
        &self,
        nonce: Nonce,
        associated_data: &[u8],
        buffer: &mut [u8],
    ) -> Result<Tag, AeadError> {
        let tag = self
            .0
            .encrypt_inout_detached(&nonce.0.into(), associated_data, buffer.into())
            .map_err(|_| AeadError)?;
        Ok(Tag(tag.into()))
    }

    /// Decrypts `buffer` in place after checking `tag`. On failure `buffer` is unspecified.
    pub fn open_in_place(
        &self,
        nonce: Nonce,
        associated_data: &[u8],
        buffer: &mut [u8],
        tag: &Tag,
    ) -> Result<(), AeadError> {
        self.0
            .decrypt_inout_detached(
                &nonce.0.into(),
                associated_data,
                buffer.into(),
                &tag.0.into(),
            )
            .map_err(|_| AeadError)
    }

    /// Encrypts `plaintext`, returning `ciphertext || tag`.
    pub fn seal(
        &self,
        nonce: Nonce,
        associated_data: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, AeadError> {
        let mut out = Vec::with_capacity(plaintext.len().saturating_add(TAG_LEN));
        out.extend_from_slice(plaintext);
        let tag = self.seal_in_place(nonce, associated_data, &mut out)?;
        out.extend_from_slice(&tag.0);
        Ok(out)
    }

    /// Decrypts `ciphertext || tag`, returning the plaintext.
    pub fn open(
        &self,
        nonce: Nonce,
        associated_data: &[u8],
        sealed: &[u8],
    ) -> Result<Vec<u8>, AeadError> {
        let split = sealed.len().checked_sub(TAG_LEN).ok_or(AeadError)?;
        let (ciphertext, tag) = sealed.split_at(split);
        let tag: [u8; TAG_LEN] = tag.try_into().map_err(|_| AeadError)?;
        let mut out = ciphertext.to_vec();
        self.open_in_place(nonce, associated_data, &mut out, &Tag(tag))?;
        Ok(out)
    }
}

impl fmt::Debug for AeadKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AeadKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use proptest::prelude::*;

    use super::{AeadError, AeadKey, Nonce, TAG_LEN};

    fn key() -> AeadKey {
        AeadKey::from_bytes(&[5; 32])
    }

    #[test]
    fn counter_nonce_layout() {
        let nonce = Nonce::from_counter(0x0102_0304_0506_0708);
        assert_eq!(nonce.0, [0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn rejects_tampering_and_wrong_context() {
        let nonce = Nonce::from_counter(1);
        let mut sealed = key().seal(nonce, b"aad", b"secret").unwrap();
        assert_eq!(sealed.len(), 6 + TAG_LEN);
        assert_eq!(key().open(nonce, b"aad", &sealed).unwrap(), b"secret");
        assert_eq!(key().open(nonce, b"bad", &sealed), Err(AeadError));
        assert_eq!(
            key().open(Nonce::from_counter(2), b"aad", &sealed),
            Err(AeadError)
        );
        sealed[0] ^= 1;
        assert_eq!(key().open(nonce, b"aad", &sealed), Err(AeadError));
        assert_eq!(key().open(nonce, b"aad", &[0; 3]), Err(AeadError));
    }

    proptest! {
        #[test]
        fn round_trips(counter in any::<u64>(), data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let nonce = Nonce::from_counter(counter);
            let sealed = key().seal(nonce, &[], &data).unwrap();
            prop_assert_eq!(key().open(nonce, &[], &sealed).unwrap(), data);
        }
    }
}
