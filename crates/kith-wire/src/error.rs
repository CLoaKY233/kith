//! The single error type for every wire operation.

/// Why bytes could not be encoded or decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WireError {
    /// Input ended before the value was complete.
    #[error("input truncated")]
    Truncated,
    /// A varint used more bytes than necessary, or exceeded 64 bits.
    #[error("non-canonical or overflowing varint")]
    BadVarint,
    /// The input does not start with the kith magic bytes.
    #[error("bad magic bytes")]
    BadMagic,
    /// The message type is not the one the caller expected.
    #[error("unexpected message type {found} (expected {expected})")]
    UnexpectedType {
        /// The type the caller asked for.
        expected: u8,
        /// The type found on the wire.
        found: u8,
    },
    /// The message is newer than this build understands.
    #[error("unsupported version {found} (newest supported is {supported})")]
    UnsupportedVersion {
        /// The version found on the wire.
        found: u64,
        /// The newest version this build understands.
        supported: u64,
    },
    /// A length exceeded the configured limit.
    #[error("length {len} exceeds limit {max}")]
    TooLarge {
        /// The declared length.
        len: u64,
        /// The limit.
        max: u64,
    },
    /// Bytes were left over after a complete value.
    #[error("trailing bytes after value")]
    TrailingBytes,
    /// Extension tags were not strictly increasing.
    #[error("extensions not in canonical order")]
    UnorderedExtensions,
    /// The body could not be encoded.
    #[error("encoding failed")]
    Encode,
    /// The body did not match the expected structure.
    #[error("malformed body")]
    Decode,
}
