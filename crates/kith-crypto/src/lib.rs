//! The narrow cryptography facade: Ed25519, X25519, ChaCha20-Poly1305 and BLAKE3.
//!
//! Plane: foundation. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! No other crate touches a raw cryptography library. This keeps the cryptographic surface
//! small enough to audit, and makes misuse hard:
//!
//! - Every signature and hash is bound to a [`Context`], so a signature made for one
//!   purpose can never be replayed as another.
//! - Secret types zeroize on drop, print as `<redacted>`, and are not `Clone`.
//! - Verification is strict: malleable signatures and weak keys are rejected.
//! - Key agreement rejects non-contributory (low-order) public keys.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod aead;
mod agree;
mod context;
mod hash;
mod random;
mod sign;

pub use aead::{AeadError, AeadKey, Nonce, TAG_LEN, Tag};
pub use agree::{AgreementError, AgreementPublic, AgreementSecret, SharedSecret};
pub use context::Context;
pub use hash::{Hash, Hasher, derive_key};
pub use random::RandomError;
#[cfg(feature = "getrandom")]
pub use random::fill_random;
pub use sign::{Signature, SignatureError, SigningKey, VerifyingKey};
