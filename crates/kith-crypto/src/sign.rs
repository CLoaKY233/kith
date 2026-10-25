//! Ed25519 signatures over domain-separated digests.
//!
//! A signature never covers raw bytes. It covers `BLAKE3-derive-key(context, message)`, so
//! the same message signed under two contexts yields two unrelated signatures.

use core::fmt;

use ed25519_dalek::Signer as _;
use kith_types::NodeId;
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeTuple as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

use crate::{Context, Hash};

/// A signature or key failed verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// The bytes are not a valid Ed25519 public key, or the key is weak.
    #[error("invalid public key")]
    InvalidKey,
    /// The signature does not match the key and message.
    #[error("signature verification failed")]
    Invalid,
}

/// A device's long-term secret signing key.
///
/// Zeroizes on drop. Not `Clone`: there is exactly one copy in memory.
pub struct SigningKey(ed25519_dalek::SigningKey);

impl SigningKey {
    /// Builds a key from a 32-byte secret seed.
    #[must_use]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(seed))
    }

    /// Generates a fresh key from the operating system's random source.
    #[cfg(feature = "getrandom")]
    pub fn generate() -> Result<Self, crate::RandomError> {
        let mut seed = Zeroizing::new([0_u8; 32]);
        crate::fill_random(seed.as_mut())?;
        Ok(Self::from_seed(&seed))
    }

    /// The secret seed, for persisting the key. Zeroizes on drop.
    #[must_use]
    pub fn to_seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The matching public key.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        VerifyingKey(self.0.verifying_key())
    }

    /// The identity this key signs for.
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        self.verifying_key().node_id()
    }

    /// Signs `message` under `context`.
    #[must_use]
    pub fn sign(&self, context: Context, message: &[u8]) -> Signature {
        let digest = Hash::of(context, message);
        Signature(self.0.sign(digest.as_bytes()).to_bytes())
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SigningKey(<redacted>, public {:?})", self.node_id())
    }
}

/// A public verification key. Always a valid, non-weak curve point.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VerifyingKey(ed25519_dalek::VerifyingKey);

impl VerifyingKey {
    /// Parses and validates a public key.
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, SignatureError> {
        let key = ed25519_dalek::VerifyingKey::from_bytes(bytes)
            .map_err(|_| SignatureError::InvalidKey)?;
        if key.is_weak() {
            return Err(SignatureError::InvalidKey);
        }
        Ok(Self(key))
    }

    /// The key for a node identity. Fails if the identity is not a valid key.
    pub fn from_node_id(id: NodeId) -> Result<Self, SignatureError> {
        Self::from_bytes(id.as_bytes())
    }

    /// The node identity of this key.
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        NodeId::from_bytes(self.0.to_bytes())
    }

    /// The raw key bytes.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// Verifies that `signature` covers `message` under `context`.
    ///
    /// Strict: rejects malleable signatures, so each message has exactly one valid
    /// signature per key.
    pub fn verify(
        &self,
        context: Context,
        message: &[u8],
        signature: &Signature,
    ) -> Result<(), SignatureError> {
        let digest = Hash::of(context, message);
        let signature = ed25519_dalek::Signature::from_bytes(&signature.0);
        self.0
            .verify_strict(digest.as_bytes(), &signature)
            .map_err(|_| SignatureError::Invalid)
    }
}

impl fmt::Debug for VerifyingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VerifyingKey({})", self.node_id().short())
    }
}

impl Serialize for VerifyingKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_bytes().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for VerifyingKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <[u8; 32]>::deserialize(deserializer)?;
        Self::from_bytes(&bytes).map_err(de::Error::custom)
    }
}

/// An Ed25519 signature.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature([u8; 64]);

impl Signature {
    /// Wraps raw signature bytes. Validity is checked by [`VerifyingKey::verify`].
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    /// Borrows the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature(")?;
        for byte in self.0.iter().take(4) {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "..)")
    }
}

// serde has no impls for arrays longer than 32, so encode as a 64-element tuple.
// In postcard that is exactly 64 raw bytes.
impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tuple = serializer.serialize_tuple(64)?;
        for byte in &self.0 {
            tuple.serialize_element(byte)?;
        }
        tuple.end()
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SignatureVisitor;

        impl<'de> Visitor<'de> for SignatureVisitor {
            type Value = Signature;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("64 signature bytes")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Signature, A::Error> {
                let mut bytes = [0_u8; 64];
                for (index, slot) in bytes.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| de::Error::invalid_length(index, &self))?;
                }
                Ok(Signature(bytes))
            }
        }

        deserializer.deserialize_tuple(64, SignatureVisitor)
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    extern crate std;

    use alloc::format;

    use proptest::prelude::*;

    use super::{Signature, SignatureError, SigningKey, VerifyingKey};
    use crate::Context;

    const CTX: Context = Context::new("kith test sign v1");
    const OTHER: Context = Context::new("kith test sign other v1");

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_seed(&[seed; 32])
    }

    #[test]
    fn signature_binds_key_message_and_context() {
        let signer = key(1);
        let public = signer.verifying_key();
        let signature = signer.sign(CTX, b"hello");
        assert_eq!(public.verify(CTX, b"hello", &signature), Ok(()));
        assert_eq!(
            public.verify(CTX, b"hellp", &signature),
            Err(SignatureError::Invalid)
        );
        assert_eq!(
            public.verify(OTHER, b"hello", &signature),
            Err(SignatureError::Invalid)
        );
        assert_eq!(
            key(2).verifying_key().verify(CTX, b"hello", &signature),
            Err(SignatureError::Invalid)
        );
    }

    #[test]
    fn rejects_weak_and_invalid_keys() {
        // The identity point is a small-order (weak) key.
        let mut identity = [0_u8; 32];
        identity[0] = 1;
        assert_eq!(
            VerifyingKey::from_bytes(&identity),
            Err(SignatureError::InvalidKey)
        );
    }

    #[test]
    fn debug_never_reveals_the_secret() {
        let signer = key(9);
        let text = format!("{signer:?}");
        assert!(text.contains("<redacted>"));
        let seed_hex =
            signer
                .to_seed()
                .iter()
                .fold(alloc::string::String::new(), |mut hex, byte| {
                    use core::fmt::Write as _;
                    write!(hex, "{byte:02x}").unwrap();
                    hex
                });
        assert!(!text.contains(&seed_hex));
    }

    #[test]
    fn seed_round_trips() {
        let signer = key(3);
        let restored = SigningKey::from_seed(&signer.to_seed());
        assert_eq!(restored.verifying_key(), signer.verifying_key());
    }

    #[test]
    fn serde_round_trips_and_validates() {
        let signer = key(4);
        let public = signer.verifying_key();
        let signature = signer.sign(CTX, b"m");

        let bytes = postcard::to_allocvec(&signature).unwrap();
        assert_eq!(bytes.len(), 64);
        assert_eq!(
            postcard::from_bytes::<Signature>(&bytes).unwrap(),
            signature
        );

        let bytes = postcard::to_allocvec(&public).unwrap();
        assert_eq!(
            postcard::from_bytes::<VerifyingKey>(&bytes).unwrap(),
            public
        );

        let mut weak = [0_u8; 32];
        weak[0] = 1;
        let bytes = postcard::to_allocvec(&weak).unwrap();
        assert!(postcard::from_bytes::<VerifyingKey>(&bytes).is_err());
    }

    proptest! {
        #[test]
        fn any_message_round_trips(seed in any::<[u8; 32]>(), message in proptest::collection::vec(any::<u8>(), 0..256)) {
            let signer = SigningKey::from_seed(&seed);
            let signature = signer.sign(CTX, &message);
            prop_assert_eq!(signer.verifying_key().verify(CTX, &message, &signature), Ok(()));
        }

        #[test]
        fn any_bit_flip_fails(seed in any::<[u8; 32]>(), flip in 0_usize..64 * 8) {
            let signer = SigningKey::from_seed(&seed);
            let signature = signer.sign(CTX, b"fixed message");
            let mut bytes = *signature.as_bytes();
            bytes[flip / 8] ^= 1 << (flip % 8);
            let tampered = Signature::from_bytes(bytes);
            prop_assert!(signer.verifying_key().verify(CTX, b"fixed message", &tampered).is_err());
        }
    }
}
