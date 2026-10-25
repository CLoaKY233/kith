//! The one error type for the protocol core.
//!
//! Variants say what was wrong with a datagram without carrying attacker-controlled detail.
//! Every rejected datagram is simply dropped by the caller; none of these is fatal.

/// Why an operation on a peer or device failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WgError {
    /// The datagram has an unknown type, a non-zero reserved field or the wrong length.
    #[error("malformed WireGuard message")]
    MalformedMessage,
    /// The caller's output buffer cannot hold the result. See [`crate::required_output_len`].
    #[error("output buffer too small")]
    BufferTooSmall,
    /// MAC1 did not verify: the sender does not know our public key.
    #[error("invalid MAC1")]
    InvalidMac,
    /// Authenticated decryption failed (wrong key, tampering or a forged message).
    #[error("decryption failed")]
    DecryptionFailed,
    /// The handshake initiation came from a static key we have no peer for.
    #[error("unknown peer")]
    UnknownPeer,
    /// No live handshake or session matches the receiver index.
    #[error("unknown receiver index")]
    UnknownIndex,
    /// A handshake timestamp or transport counter was already seen (or is too old).
    #[error("replayed message")]
    Replay,
    /// The session behind the receiver index has expired.
    #[error("session expired")]
    SessionExpired,
    /// The decrypted payload is not a well-formed IPv4 or IPv6 packet.
    #[error("invalid inner IP packet")]
    InvalidPacket,
    /// Diffie-Hellman with a peer key produced a non-contributory (low-order) result.
    #[error("low-order public key")]
    LowOrderKey,
    /// The random source failed.
    #[error("random source failed")]
    RandomFailed,
    /// The device ran out of peer slots or the peer already exists.
    #[error("peer cannot be added")]
    PeerRejected,
}
