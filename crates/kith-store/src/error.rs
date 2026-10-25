//! Error types.

use std::{io, path::PathBuf};

/// Result alias for store operations.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// A store operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A filesystem call failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// What was being done, e.g. `"rename"`.
        op: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// A stored file failed validation (torn write, truncation, bit rot).
    #[error("corrupt value at {}: {reason}", path.display())]
    Corrupt {
        /// The corrupt file.
        path: PathBuf,
        /// What check failed.
        reason: &'static str,
    },
    /// A key, name or namespace was rejected.
    #[error("invalid key: {0}")]
    InvalidKey(#[from] KeyError),
    /// A value exceeds [`MAX_VALUE_LEN`](crate::MAX_VALUE_LEN).
    #[error("value of {len} bytes exceeds the {max}-byte limit")]
    TooLarge {
        /// Length of the rejected value.
        len: usize,
        /// The limit.
        max: usize,
    },
}

/// Why a key component was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// An empty name or namespace segment.
    #[error("empty segment")]
    Empty,
    /// A segment or namespace is longer than allowed.
    #[error("too long")]
    TooLong,
    /// A byte outside `[a-z0-9._-]` (or `/` outside a namespace).
    #[error("byte {0:#04x} not allowed")]
    InvalidByte(u8),
    /// A segment starts with `.` (covers `.` and `..`).
    #[error("segment starts with a dot")]
    LeadingDot,
}

impl Error {
    pub(crate) fn io(op: &'static str, path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { op, path, source }
    }
}
