//! The CLI side.

use std::path::Path;

use tokio::sync::mpsc;

use crate::codec::Framed;
use crate::message::ServerMessage;
use crate::{Event, IpcError, PROTOCOL_VERSION, REQUEST_TIMEOUT, Request, Response};

#[cfg(unix)]
type Stream = tokio::net::UnixStream;
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

/// A connection to the daemon.
#[derive(Debug)]
pub struct IpcClient {
    framed: Framed<Stream>,
    daemon_version: String,
}

impl IpcClient {
    /// Connects to the daemon at `path` and negotiates the protocol version.
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, IpcError> {
        #[cfg(unix)]
        let stream = Stream::connect(path.as_ref()).await?;
        #[cfg(windows)]
        let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(path.as_ref())?;
        let mut client = Self {
            framed: Framed::new(stream),
            daemon_version: String::new(),
        };
        let hello = Request::Hello {
            protocol: PROTOCOL_VERSION,
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "any other response breaks the handshake"
        )]
        match client.request(hello).await? {
            Response::Hello {
                protocol: PROTOCOL_VERSION,
                daemon_version,
            } => client.daemon_version = daemon_version,
            Response::Hello { protocol, .. } => {
                return Err(IpcError::VersionMismatch {
                    ours: PROTOCOL_VERSION,
                    theirs: protocol,
                });
            }
            _ => return Err(IpcError::Unexpected),
        }
        Ok(client)
    }

    /// The daemon's software version.
    pub fn daemon_version(&self) -> &str {
        &self.daemon_version
    }

    /// Sends `request` and waits for its response. [`Response::Error`] becomes an error.
    pub async fn request(&mut self, request: Request) -> Result<Response, IpcError> {
        tokio::time::timeout(REQUEST_TIMEOUT, self.exchange(&request))
            .await
            .map_err(|_| IpcError::Timeout)?
    }

    /// Subscribes to daemon events. The channel closes when the connection ends.
    pub async fn watch(mut self) -> Result<mpsc::Receiver<Result<Event, IpcError>>, IpcError> {
        if self.request(Request::Watch).await? != Response::Watching {
            return Err(IpcError::Unexpected);
        }
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                let item = match self.framed.recv::<ServerMessage>().await {
                    Ok(Some(ServerMessage::Event(event))) => Ok(event),
                    Ok(None) => break,
                    Ok(Some(ServerMessage::Response(_))) => Err(IpcError::Unexpected),
                    Err(error) => Err(error),
                };
                let fatal = item.is_err();
                if tx.send(item).await.is_err() || fatal {
                    break;
                }
            }
        });
        Ok(rx)
    }

    async fn exchange(&mut self, request: &Request) -> Result<Response, IpcError> {
        self.framed.send(request).await?;
        match self.framed.recv::<ServerMessage>().await? {
            Some(ServerMessage::Response(Response::Error { code, message })) => {
                Err(IpcError::Remote { code, message })
            }
            Some(ServerMessage::Response(response)) => Ok(response),
            Some(ServerMessage::Event(_)) => Err(IpcError::Unexpected),
            None => Err(IpcError::Closed),
        }
    }
}
