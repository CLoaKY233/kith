//! Device identity, key storage abstraction and invite tokens.
//!
//! Sans-IO. kith has no accounts: a device *is* its Ed25519 key ([`DeviceIdentity`]),
//! persisted through a [`KeyStore`] implemented by driver crates, and a network is joined
//! by holding an [`InviteToken`]. Verifying join proofs is the ledger's job, so this crate
//! does not depend on `kith-ledger`.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

mod bootstrap;
pub mod contexts;
mod identity;
mod invite;
mod keystore;
pub mod message_types;

pub use bootstrap::{Bootstrap, BootstrapError, Hostname, MAX_HOSTNAME_LEN};
pub use identity::{DeviceIdentity, IdentityError, SECRET_KEY_VERSION};
pub use invite::{
    INVITE_VERSION, InviteError, InviteToken, LINK_PREFIX, MAX_BOOTSTRAP, MAX_LINK_PAYLOAD_LEN,
    MAX_NETWORK_NAME_LEN,
};
pub use keystore::{KeyStore, KeyStoreError, MemoryKeyStore};
