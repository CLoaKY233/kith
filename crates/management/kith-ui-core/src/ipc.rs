//! The only daemon client: every read and change the page makes goes through here, to the
//! daemon's socket or, under `--fixture`, to a recorded network.

use std::path::PathBuf;

use axum::http::StatusCode;
use kith_ipc::{ErrorCode, IpcClient, IpcError, Request, Response};

use crate::Source as Config;
use crate::api::ApiError;
use crate::wire::{ApiErrorCode, Fixture};

/// The daemon behind its socket.
#[derive(Debug)]
pub(crate) struct Daemon {
    socket: PathBuf,
}

/// Where the page's data comes from.
#[derive(Debug)]
pub(crate) enum Source {
    Daemon(Daemon),
    Fixture(Box<Fixture>),
}

impl Source {
    pub(crate) fn new(config: Config) -> Self {
        match config {
            Config::Daemon(socket) => Self::Daemon(Daemon { socket }),
            Config::Fixture(fixture) => Self::Fixture(fixture),
        }
    }
}

impl Daemon {
    /// A new connection, after the protocol handshake.
    pub(crate) async fn connect(&self) -> Result<IpcClient, ApiError> {
        IpcClient::connect(&self.socket).await.map_err(|error| {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "every other failure means the daemon is not answering"
            )]
            let code = match &error {
                IpcError::VersionMismatch { .. }
                | IpcError::Remote {
                    code: ErrorCode::VersionMismatch,
                    ..
                } => ApiErrorCode::VersionSkew,
                IpcError::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
                    ApiErrorCode::NotInGroup
                }
                _ => ApiErrorCode::DaemonDown,
            };
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, code, error.to_string())
        })
    }

    /// One request on a fresh connection.
    pub(crate) async fn ask(&self, request: Request) -> Result<Response, ApiError> {
        Ok(self.connect().await?.request(request).await?)
    }
}
