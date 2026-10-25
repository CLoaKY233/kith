//! Where a device keeps its identity. Backends live in driver crates.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;

use zeroize::Zeroizing;

use crate::{DeviceIdentity, IdentityError};

/// Why a key store operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KeyStoreError {
    /// The stored bytes are not a valid secret-key file.
    #[error("stored identity is invalid: {0}")]
    Corrupt(#[from] IdentityError),
    /// The backend refused access (locked keychain, wrong permissions, user denied, …).
    #[error("{backend}: access denied: {detail}")]
    AccessDenied {
        /// The backend, for example `"file"` or `"macos-keychain"`.
        backend: &'static str,
        /// What the backend reported.
        detail: Cow<'static, str>,
    },
    /// Any other backend failure.
    #[error("{backend}: {detail}")]
    Backend {
        /// The backend, for example `"file"` or `"macos-keychain"`.
        backend: &'static str,
        /// What the backend reported.
        detail: Cow<'static, str>,
    },
}

/// Persists the device identity as the bytes of [`DeviceIdentity::to_secret_bytes`].
pub trait KeyStore {
    /// Loads the stored identity, or `None` if none has been stored yet.
    fn load(&self) -> Result<Option<DeviceIdentity>, KeyStoreError>;

    /// Stores `identity`, replacing any previous one.
    fn store(&mut self, identity: &DeviceIdentity) -> Result<(), KeyStoreError>;
}

/// A key store held in memory, for tests and ephemeral nodes. Zeroizes on drop.
#[derive(Default)]
pub struct MemoryKeyStore {
    secret: Option<Zeroizing<Vec<u8>>>,
}

impl MemoryKeyStore {
    /// An empty store.
    #[must_use]
    pub const fn new() -> Self {
        Self { secret: None }
    }

    /// A store holding raw bytes, for testing how corrupt data is handled.
    #[must_use]
    pub fn with_secret_bytes(bytes: &[u8]) -> Self {
        Self {
            secret: Some(Zeroizing::new(bytes.to_vec())),
        }
    }
}

impl KeyStore for MemoryKeyStore {
    fn load(&self) -> Result<Option<DeviceIdentity>, KeyStoreError> {
        self.secret
            .as_deref()
            .map(|bytes| DeviceIdentity::from_secret_bytes(bytes).map_err(KeyStoreError::from))
            .transpose()
    }

    fn store(&mut self, identity: &DeviceIdentity) -> Result<(), KeyStoreError> {
        self.secret = Some(identity.to_secret_bytes());
        Ok(())
    }
}

impl fmt::Debug for MemoryKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = if self.secret.is_some() {
            "<redacted>"
        } else {
            "empty"
        };
        f.debug_struct("MemoryKeyStore")
            .field("secret", &state)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use alloc::borrow::Cow;
    use alloc::format;
    use alloc::string::ToString;

    use super::{KeyStore, KeyStoreError, MemoryKeyStore};
    use crate::{DeviceIdentity, IdentityError};

    #[test]
    fn stores_and_loads() {
        let mut store = MemoryKeyStore::new();
        assert!(store.load().unwrap().is_none());
        let identity = DeviceIdentity::from_seed(&[3; 32]);
        store.store(&identity).unwrap();
        assert_eq!(store.load().unwrap().unwrap().node_id(), identity.node_id());
        let replacement = DeviceIdentity::from_seed(&[4; 32]);
        store.store(&replacement).unwrap();
        assert_eq!(
            store.load().unwrap().unwrap().node_id(),
            replacement.node_id()
        );
    }

    #[test]
    fn reports_corruption() {
        let mut bytes = DeviceIdentity::from_seed(&[3; 32])
            .to_secret_bytes()
            .to_vec();
        bytes[10] ^= 0xff;
        let store = MemoryKeyStore::with_secret_bytes(&bytes);
        assert_eq!(
            store.load().err(),
            Some(KeyStoreError::Corrupt(IdentityError::BadChecksum))
        );
    }

    #[test]
    fn debug_hides_the_secret() {
        let mut store = MemoryKeyStore::default();
        assert!(format!("{store:?}").contains("empty"));
        store.store(&DeviceIdentity::from_seed(&[3; 32])).unwrap();
        let text = format!("{store:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("3, 3"));
    }

    #[test]
    fn backend_errors_carry_a_description() {
        let error = KeyStoreError::Backend {
            backend: "file",
            detail: Cow::Borrowed("disk full"),
        };
        assert_eq!(error.to_string(), "file: disk full");
        let denied = KeyStoreError::AccessDenied {
            backend: "macos-keychain",
            detail: Cow::Owned("user cancelled".into()),
        };
        assert_eq!(
            denied.to_string(),
            "macos-keychain: access denied: user cancelled"
        );
    }
}
