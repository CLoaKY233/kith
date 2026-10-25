//! State directory, atomic writes and ledger persistence.
//!
//! Plane: management. Library. See `docs/architecture.md`.
//!
//! - [`Store`] is a small synchronous key-value trait. Keys are a validated
//!   [`Namespace`] plus a validated [`Name`]; the character set is so narrow
//!   that a key can never name a path outside the store (no `..`, no
//!   separators other than the namespace `/`, no leading dots, lowercase
//!   ASCII only).
//! - [`FsStore`] persists each value in its own file using
//!   write-temp → `fsync` → `rename` → `fsync(dir)`, so a crash leaves
//!   either the old value or the new one. Every file carries a header with a
//!   CRC-32, so torn or bit-rotted data is reported as
//!   [`Error::Corrupt`] instead of being returned.
//! - [`MemoryStore`] implements the same trait in memory, for tests.
//!
//! The format below is pinned by a golden-bytes test; changing it is a format break.
//!
//! # On-disk format
//!
//! Layout: the key `ledger/ab12` + `ops` lives at `<root>/ledger/ab12/@ops`.
//! Namespace segments are directories; value files carry an `@` prefix,
//! which cannot appear in a segment, so a value never collides with a
//! sub-namespace. Temporary files are named `.tmp.@<name>.<pid>.<counter>`;
//! the leading dot cannot appear in a valid name, and [`FsStore::open`]
//! deletes any such leftovers from earlier crashes.
//!
//! Each value file is a 16-byte header followed by the payload:
//!
//! | offset | size | field                                           |
//! |-------:|-----:|-------------------------------------------------|
//! | 0      | 4    | magic `b"KITH"`                                 |
//! | 4      | 1    | format version, currently `1`                   |
//! | 5      | 3    | reserved, must be zero                          |
//! | 8      | 4    | payload length, `u32` little-endian             |
//! | 12     | 4    | CRC-32 (IEEE, reflected `0xEDB88320`) of bytes `0..12` followed by the payload, `u32` little-endian |
//! | 16     | len  | payload                                         |
//!
//! A file whose length does not match the header (torn write, truncation)
//! or whose checksum does not match is corrupt.
//!
//! # Platforms
//!
//! On Unix, secret values ([`Secrecy::Secret`]) are created with mode
//! `0600` and any directory created for them with `0700`, atomically at
//! creation. On Windows, `std::fs::rename` maps to `MoveFileExW` with
//! `MOVEFILE_REPLACE_EXISTING`, which replaces the target atomically on
//! NTFS; directories cannot be opened for `fsync` with safe std APIs, so the
//! directory flush is skipped and file permissions follow the inherited ACL
//! of the state directory.

#![forbid(unsafe_code)]

mod crc;
mod error;
mod format;
mod fs;
mod key;
mod memory;

#[cfg(test)]
mod tests;

pub use error::{Error, KeyError, Result};
pub use format::MAX_VALUE_LEN;
pub use fs::FsStore;
pub use key::{Key, MAX_NAMESPACE_LEN, MAX_SEGMENT_LEN, Name, Namespace};
pub use memory::MemoryStore;

/// Whether a value is sensitive (device keys) or ordinary state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Secrecy {
    /// Ordinary state, readable by other local users (Unix mode `0644`).
    #[default]
    Public,
    /// Owner-only: files `0600` and newly created directories `0700` on Unix.
    Secret,
}

/// A synchronous, durable key-value store.
///
/// Values are at most [`MAX_VALUE_LEN`] bytes. A successful `put` or
/// `delete` is durable when it returns.
pub trait Store: Send + Sync {
    /// Reads the value under `key`, or `None` if there is none.
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>>;

    /// Atomically replaces the value under `key`.
    fn put_with(&self, key: &Key, value: &[u8], secrecy: Secrecy) -> Result<()>;

    /// Removes the value under `key`. Removing a missing key succeeds.
    fn delete(&self, key: &Key) -> Result<()>;

    /// Lists the keys directly in `namespace` (not in sub-namespaces), sorted.
    fn list(&self, namespace: &Namespace) -> Result<Vec<Key>>;

    /// Stores an ordinary value; see [`Store::put_with`].
    fn put(&self, key: &Key, value: &[u8]) -> Result<()> {
        self.put_with(key, value, Secrecy::Public)
    }

    /// Stores a secret value; see [`Secrecy::Secret`].
    fn put_secret(&self, key: &Key, value: &[u8]) -> Result<()> {
        self.put_with(key, value, Secrecy::Secret)
    }
}
