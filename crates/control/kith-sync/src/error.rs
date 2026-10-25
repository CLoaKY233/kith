//! Why a sync message was refused.

use kith_types::NetworkId;
use kith_wire::WireError;

/// A sync failure. Every variant except [`LedgerMismatch`](Self::LedgerMismatch) ends the
/// session: the peer broke the protocol, so the driver should drop it and retry later.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SyncError {
    /// The envelope could not be decoded.
    #[error("bad envelope: {0}")]
    Wire(#[from] WireError),
    /// The envelope carries a version this crate does not speak.
    #[error("unsupported sync version {0}")]
    UnsupportedVersion(u64),
    /// The message is larger than [`MAX_SYNC_MESSAGE_LEN`](crate::MAX_SYNC_MESSAGE_LEN).
    #[error("sync message of {0} bytes is too large")]
    TooLarge(usize),
    /// A list in the message exceeds its documented bound.
    #[error("sync message exceeds a size limit")]
    LimitExceeded,
    /// A `Message::Ranges` is not a sorted partition of the id
    /// space, or an id list holds ids outside its range or out of order.
    #[error("malformed ranges")]
    MalformedRanges,
    /// The peer's `Hello` names another network.
    #[error("peer is syncing network {found}, expected {expected}")]
    WrongNetwork {
        /// This session's network.
        expected: NetworkId,
        /// The network the peer named.
        found: NetworkId,
    },
    /// The ledger passed in belongs to another network than the session. This is a driver
    /// bug; the session is left untouched.
    #[error("ledger belongs to another network than the session")]
    LedgerMismatch,
    /// The message is not allowed at this point of the protocol.
    #[error("unexpected sync message")]
    UnexpectedMessage,
    /// The peer kept the session going for more than `MAX_ROUNDS`.
    #[error("too many sync rounds")]
    TooManyRounds,
    /// The session already finished or failed.
    #[error("sync session is closed")]
    Closed,
}
