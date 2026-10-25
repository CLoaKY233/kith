//! Parse errors shared by every view.

/// Why a buffer was rejected by a `new_checked` constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The buffer is shorter than the fixed header or a declared length.
    #[error("packet truncated")]
    Truncated,
    /// The IP version nibble is not the one expected (or neither 4 nor 6).
    #[error("unsupported IP version")]
    BadVersion,
    /// A header-length field (IPv4 IHL, TCP data offset) is below the minimum
    /// or exceeds the packet.
    #[error("bad header length")]
    BadHeaderLength,
    /// A total-length field (IPv4 total length, UDP length) is inconsistent.
    #[error("bad total length")]
    BadTotalLength,
}
