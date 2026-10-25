use kith_ledger::LedgerError;
use kith_sync::SyncError;
use kith_wire::WireError;

/// A control-channel failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChannelError {
    /// A socket operation failed.
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// The TLS configuration could not be built.
    #[error("tls: {0}")]
    Tls(#[from] kith_tls::Error),
    /// A message could not be encoded or decoded.
    #[error("wire: {0}")]
    Wire(#[from] WireError),
    /// A ledger sync session failed.
    #[error("sync: {0}")]
    Sync(#[from] SyncError),
    /// The sponsor's genesis op did not open a ledger.
    #[error("ledger: {0}")]
    Ledger(#[from] LedgerError),
    /// A message exceeds one of its documented bounds.
    #[error("message exceeds a size limit")]
    LimitExceeded,
    /// The peer sent a message the protocol does not allow at this point.
    #[error("unexpected message")]
    UnexpectedMessage,
    /// The peer shares no network with this node, or a join did not make it a member.
    #[error("not a member")]
    NotMember,
    /// The sponsor refused the join, for the reason given.
    #[error("join rejected: {0}")]
    JoinRejected(String),
    /// The peer did not answer in time.
    #[error("timed out")]
    TimedOut,
    /// The peer closed the connection.
    #[error("connection closed")]
    Closed,
}
