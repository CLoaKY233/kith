//! A device's long-term identity and its secret-key file format.
//!
//! The file format is deliberately tiny and strict: a kith envelope holding the 32-byte seed
//! and a BLAKE3 checksum of it. The checksum catches storage corruption (a flipped bit would
//! otherwise silently become a *different* identity). It is not authentication: whoever can
//! write the file can replace the key, so protecting the file is the store's job.

use alloc::vec::Vec;
use core::fmt;

use kith_crypto::{Hash, SigningKey, VerifyingKey};
use kith_types::NodeId;
use kith_wire::{WireError, envelope};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::{contexts, message_types};

/// The current (and only) secret-key file version.
pub const SECRET_KEY_VERSION: u64 = 1;

/// Why a secret-key file was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityError {
    /// The bytes are not a well-formed secret-key envelope of a supported version.
    #[error("malformed secret key file: {0}")]
    Wire(#[from] WireError),
    /// The file is a secret-key envelope of a version this build does not accept.
    #[error("unsupported secret key file version {0}")]
    UnsupportedVersion(u64),
    /// The checksum does not match the seed: the file is corrupt.
    #[error("secret key checksum mismatch")]
    BadChecksum,
    /// The operating system's random source failed.
    #[error("random source unavailable")]
    Random,
}

/// The device's long-term Ed25519 identity. Its public key is the device's [`NodeId`].
///
/// Not `Clone`: there is exactly one copy of the secret in memory. `Debug` prints only the
/// public node id. The key zeroizes on drop.
pub struct DeviceIdentity {
    signing_key: SigningKey,
}

/// The postcard body of a secret-key file. Zeroizes on drop.
#[derive(Serialize, Deserialize)]
struct SecretKeyBody {
    seed: [u8; 32],
    checksum: [u8; 32],
}

impl Drop for SecretKeyBody {
    fn drop(&mut self) {
        self.seed.zeroize();
    }
}

fn seed_checksum(seed: &[u8; 32]) -> [u8; 32] {
    Hash::of(contexts::SECRET_KEY_CHECKSUM, seed).to_bytes()
}

impl DeviceIdentity {
    /// Builds an identity from a 32-byte secret seed.
    #[must_use]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_seed(seed),
        }
    }

    /// Generates a fresh identity from the operating system's random source.
    #[cfg(feature = "getrandom")]
    pub fn generate() -> Result<Self, IdentityError> {
        let signing_key = SigningKey::generate().map_err(|_| IdentityError::Random)?;
        Ok(Self { signing_key })
    }

    /// The device's public identifier.
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        self.signing_key.node_id()
    }

    /// The device's public key.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Borrows the secret signing key.
    #[must_use]
    pub const fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    /// Encodes the secret-key file: a version-1 [`message_types::SECRET_KEY`] envelope
    /// holding the seed and its checksum. Zeroizes on drop.
    ///
    /// # Panics
    ///
    /// Never: the body has a fixed size, so encoding cannot fail.
    pub fn to_secret_bytes(&self) -> Zeroizing<Vec<u8>> {
        let seed = self.signing_key.to_seed();
        let body = SecretKeyBody {
            seed: *seed,
            checksum: seed_checksum(&seed),
        };
        #[expect(
            clippy::expect_used,
            reason = "encoding a fixed 64-byte body cannot fail or exceed the size limit"
        )]
        let bytes = envelope::encode(message_types::SECRET_KEY, SECRET_KEY_VERSION, &body)
            .expect("a fixed-size secret key body always encodes");
        Zeroizing::new(bytes)
    }

    /// Decodes a secret-key file, rejecting any other message type or version, trailing
    /// bytes and a checksum mismatch.
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
        let (version, body): (u64, SecretKeyBody) =
            envelope::decode(bytes, message_types::SECRET_KEY, SECRET_KEY_VERSION)?;
        if version != SECRET_KEY_VERSION {
            return Err(IdentityError::UnsupportedVersion(version));
        }
        if seed_checksum(&body.seed) != body.checksum {
            return Err(IdentityError::BadChecksum);
        }
        Ok(Self::from_seed(&body.seed))
    }
}

impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("node_id", &self.node_id())
            .field("signing_key", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::format;
    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::{DeviceIdentity, IdentityError};
    use crate::message_types;
    use kith_wire::{WireError, envelope};

    const SEED: [u8; 32] = [7; 32];

    fn hex(bytes: &[u8]) -> std::string::String {
        use core::fmt::Write as _;
        let mut out = std::string::String::new();
        for byte in bytes {
            write!(out, "{byte:02x}").unwrap();
        }
        out
    }

    #[test]
    fn golden_bytes() {
        // Pins the format: changing these bytes is a protocol break.
        let bytes = DeviceIdentity::from_seed(&SEED).to_secret_bytes();
        assert_eq!(hex(&bytes), GOLDEN);
    }

    /// magic "kt", type 0x20, version 1, the seed, then its checksum.
    const GOLDEN: &str = concat!(
        "6b742001",
        "0707070707070707070707070707070707070707070707070707070707070707",
        "cb2edbccb6a2260bcbcc0716633b299fbdb1386b076acbc80a0e51711a2bc4a1",
    );

    #[test]
    fn round_trips() {
        let identity = DeviceIdentity::from_seed(&SEED);
        let restored = DeviceIdentity::from_secret_bytes(&identity.to_secret_bytes()).unwrap();
        assert_eq!(restored.node_id(), identity.node_id());
        assert_eq!(*restored.signing_key().to_seed(), SEED);
        assert_eq!(identity.verifying_key().node_id(), identity.node_id());
    }

    #[test]
    fn rejects_corruption() {
        let bytes = DeviceIdentity::from_seed(&SEED).to_secret_bytes();
        let mut flipped = bytes.to_vec();
        flipped[5] ^= 1;
        assert_eq!(
            DeviceIdentity::from_secret_bytes(&flipped).err(),
            Some(IdentityError::BadChecksum)
        );
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert_eq!(
            DeviceIdentity::from_secret_bytes(&trailing).err(),
            Some(IdentityError::Wire(WireError::TrailingBytes))
        );
        assert_eq!(
            DeviceIdentity::from_secret_bytes(&bytes[..bytes.len() - 1]).err(),
            Some(IdentityError::Wire(WireError::Truncated))
        );
        let mut wrong_type = bytes.to_vec();
        wrong_type[2] = message_types::INVITE.0;
        assert!(matches!(
            DeviceIdentity::from_secret_bytes(&wrong_type),
            Err(IdentityError::Wire(WireError::UnexpectedType { .. }))
        ));
    }

    #[test]
    fn rejects_other_versions() {
        let body = ([1_u8; 32], [2_u8; 32]);
        let future = envelope::encode(message_types::SECRET_KEY, 2, &body).unwrap();
        assert!(matches!(
            DeviceIdentity::from_secret_bytes(&future),
            Err(IdentityError::Wire(WireError::UnsupportedVersion { .. }))
        ));
        let old = envelope::encode(message_types::SECRET_KEY, 0, &body).unwrap();
        assert_eq!(
            DeviceIdentity::from_secret_bytes(&old).err(),
            Some(IdentityError::UnsupportedVersion(0))
        );
    }

    #[test]
    fn debug_hides_the_seed() {
        let identity = DeviceIdentity::from_seed(&SEED);
        let text = format!("{identity:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains(&hex(&SEED)));
        assert!(!text.contains("7, 7"));
    }

    #[cfg(feature = "getrandom")]
    #[test]
    fn generated_identities_differ() {
        let first = DeviceIdentity::generate().unwrap();
        let second = DeviceIdentity::generate().unwrap();
        assert_ne!(first.node_id(), second.node_id());
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(input in proptest::collection::vec(any::<u8>(), 0..128)) {
            let _ignored = DeviceIdentity::from_secret_bytes(&input);
        }

        #[test]
        fn arbitrary_seeds_round_trip(seed in any::<[u8; 32]>()) {
            let identity = DeviceIdentity::from_seed(&seed);
            let bytes: Vec<u8> = identity.to_secret_bytes().to_vec();
            let restored = DeviceIdentity::from_secret_bytes(&bytes).unwrap();
            prop_assert_eq!(restored.node_id(), identity.node_id());
        }
    }
}
