use std::{io, path::PathBuf};

/// An OS-integration failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The interface name is not `[A-Za-z0-9._-]{1,15}`.
    #[error("invalid interface name {0:?}")]
    InvalidInterface(String),
    /// The domain is not a valid DNS name.
    #[error("invalid domain {0:?}")]
    InvalidDomain(String),
    /// A command argument contains characters outside the safe set.
    #[error("refusing unsafe command argument {0:?}")]
    InvalidArgument(String),
    /// A command exited unsuccessfully.
    #[error("`{command}` failed (exit {code:?}): {stderr}")]
    Command {
        /// The command line.
        command: String,
        /// The exit code, if it exited normally.
        code: Option<i32>,
        /// Captured stderr.
        stderr: String,
    },
    /// A command could not be started.
    #[error("cannot run `{command}`: {source}")]
    Spawn {
        /// The command line.
        command: String,
        /// The underlying error.
        source: io::Error,
    },
    /// A file operation failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// A command printed output kith could not interpret.
    #[error("unexpected output from `{command}`: {output:?}")]
    Parse {
        /// The command line.
        command: String,
        /// The offending output.
        output: String,
    },
    /// A file kith would write exists and is not kith's.
    #[error("{} exists and is not managed by kith; refusing to overwrite", .0.display())]
    Conflict(PathBuf),
    /// The operation is not supported on this platform.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}
