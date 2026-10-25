//! Device identity, key storage abstraction and invite tokens.
//!
//! Plane: trust. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! kith has no accounts. A device *is* its Ed25519 key, and a network is joined by holding
//! an invite. This crate owns the three pieces that make that work:
//!
//! - [`DeviceIdentity`]: the device's long-term signing key, with a versioned, checksummed
//!   secret-key file format ([`DeviceIdentity::to_secret_bytes`]).
//! - [`KeyStore`]: a sans-IO trait for persisting that identity. Files and OS keychains are
//!   implemented by driver crates; [`MemoryKeyStore`] serves tests.
//! - [`InviteToken`]: the bearer secret behind a `kith://join/…` link or QR code.
//!
//! This crate only *produces* keys and tokens. Verifying a join proof is the ledger's job,
//! so nothing here depends on `kith-ledger`.
//!
//! Every hash context this crate uses is listed in [`contexts`]; every envelope message
//! type in [`message_types`].

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
