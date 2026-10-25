//! X25519 key agreement.

use core::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Key agreement produced an all-zero secret: the peer sent a low-order public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("key agreement was not contributory (low-order public key)")]
pub struct AgreementError;

/// A long-term X25519 secret. Zeroizes on drop and is not `Clone`.
pub struct AgreementSecret(x25519_dalek::StaticSecret);

impl AgreementSecret {
    /// Builds a secret from 32 bytes. Clamping is applied internally.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(x25519_dalek::StaticSecret::from(bytes))
    }

    /// Generates a fresh secret from the operating system's random source.
    #[cfg(feature = "getrandom")]
    pub fn generate() -> Result<Self, crate::RandomError> {
        let mut bytes = Zeroizing::new([0_u8; 32]);
        crate::fill_random(bytes.as_mut())?;
        Ok(Self::from_bytes(*bytes))
    }

    /// The secret bytes, for persisting. Zeroizes on drop.
    #[must_use]
    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The matching public key.
    #[must_use]
    pub fn public_key(&self) -> AgreementPublic {
        AgreementPublic(x25519_dalek::PublicKey::from(&self.0).to_bytes())
    }

    /// Computes the shared secret with `peer`. Fails on low-order keys.
    pub fn agree(&self, peer: &AgreementPublic) -> Result<SharedSecret, AgreementError> {
        let shared = self
            .0
            .diffie_hellman(&x25519_dalek::PublicKey::from(peer.0));
        if !shared.was_contributory() {
            return Err(AgreementError);
        }
        Ok(SharedSecret(Zeroizing::new(shared.to_bytes())))
    }
}

impl fmt::Debug for AgreementSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AgreementSecret(<redacted>, public {:?})",
            self.public_key()
        )
    }
}

/// An X25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgreementPublic([u8; 32]);

impl AgreementPublic {
    /// Wraps raw public key bytes. Low-order keys are caught by [`AgreementSecret::agree`].
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw key bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for AgreementPublic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AgreementPublic(")?;
        for byte in self.0.iter().take(4) {
            write!(f, "{byte:02x}")?;
        }
        write!(f, ")")
    }
}

/// The output of key agreement. Zeroizes on drop. Feed it to [`crate::derive_key`]; never
/// use it as a key directly.
pub struct SharedSecret(Zeroizing<[u8; 32]>);

impl SharedSecret {
    /// Borrows the secret bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedSecret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::{AgreementError, AgreementPublic, AgreementSecret};

    #[test]
    fn both_sides_agree() {
        let alice = AgreementSecret::from_bytes([1; 32]);
        let bob = AgreementSecret::from_bytes([2; 32]);
        let a = alice.agree(&bob.public_key()).unwrap();
        let b = bob.agree(&alice.public_key()).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn rejects_low_order_points() {
        let secret = AgreementSecret::from_bytes([7; 32]);
        assert_eq!(
            secret.agree(&AgreementPublic::from_bytes([0; 32])).err(),
            Some(AgreementError)
        );
    }
}
