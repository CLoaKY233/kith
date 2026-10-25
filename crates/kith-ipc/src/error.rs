use kith_wire::WireError;

use crate::ErrorCode;

/// Why an IPC operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IpcError {
    /// The socket failed.
    #[error("ipc i/o: {0}")]
    Io(#[from] std::io::Error),
    /// A frame or message was malformed.
    #[error("ipc wire: {0}")]
    Wire(#[from] WireError),
    /// The peer closed the connection.
    #[error("connection closed")]
    Closed,
    /// No response arrived in time.
    #[error("request timed out")]
    Timeout,
    /// The two sides speak different protocol versions.
    #[error("protocol version mismatch: ours {ours}, theirs {theirs}; upgrade the older side")]
    VersionMismatch {
        /// Our version.
        ours: u32,
        /// The peer's version.
        theirs: u32,
    },
    /// The peer sent a message that does not fit the conversation.
    #[error("unexpected message from peer")]
    Unexpected,
    /// The daemon answered with an error.
    #[error("daemon error ({code:?}): {message}")]
    Remote {
        /// Machine-readable kind.
        code: ErrorCode,
        /// Human-readable explanation.
        message: String,
    },
}
