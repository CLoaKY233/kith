//! The daemon side: a Unix socket (or Windows named pipe) and per-connection handling.

use std::io;
use std::path::{Path, PathBuf};

use crate::codec::Framed;
use crate::message::ServerMessage;
use crate::{ErrorCode, Event, IpcError, PROTOCOL_VERSION, Request, Response};

#[cfg(unix)]
type Stream = tokio::net::UnixStream;
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeServer;

/// The OS identity of the process on the other end of a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCred {
    /// User id.
    pub uid: u32,
    /// Primary group id.
    pub gid: u32,
    /// Process id, where the OS reports it.
    pub pid: Option<i32>,
}

/// Listens for CLI connections.
#[derive(Debug)]
pub struct IpcServer {
    path: PathBuf,
    #[cfg(unix)]
    listener: tokio::net::UnixListener,
    #[cfg(windows)]
    next: tokio::sync::Mutex<tokio::net::windows::named_pipe::NamedPipeServer>,
}

impl IpcServer {
    /// Binds `path`, replacing a stale socket nobody is listening on. Creates its
    /// directory if missing, as `/var/run` is emptied at boot.
    #[cfg(unix)]
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};

        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o755)
                .create(dir)?;
        }
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if !meta.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "ipc path exists and is not a socket",
                ));
            }
            if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another daemon is listening on the ipc socket",
                ));
            }
            std::fs::remove_file(&path)?;
        }
        let listener = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
        Ok(Self { path, listener })
    }

    /// Creates the first instance of the named pipe `path`.
    #[cfg(windows)]
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        use tokio::net::windows::named_pipe::ServerOptions;

        let path = path.as_ref().to_path_buf();
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)?;
        Ok(Self {
            path,
            next: tokio::sync::Mutex::new(first),
        })
    }

    /// Waits for the next client.
    #[cfg(unix)]
    pub async fn accept(&self) -> Result<Connection, IpcError> {
        let (stream, _) = self.listener.accept().await?;
        let cred = stream.peer_cred()?;
        let peer = Some(PeerCred {
            uid: cred.uid(),
            gid: cred.gid(),
            pid: cred.pid(),
        });
        Ok(Connection::new(stream, peer))
    }

    /// Waits for the next client.
    #[cfg(windows)]
    pub async fn accept(&self) -> Result<Connection, IpcError> {
        use tokio::net::windows::named_pipe::ServerOptions;

        let mut next = self.next.lock().await;
        next.connect().await?;
        let fresh = ServerOptions::new().create(&self.path)?;
        let stream = core::mem::replace(&mut *next, fresh);
        Ok(Connection::new(stream, None))
    }
}

#[cfg(unix)]
impl Drop for IpcServer {
    fn drop(&mut self) {
        // Best effort: a leftover socket is replaced at the next bind anyway.
        let _ignored = std::fs::remove_file(&self.path);
    }
}

/// One CLI connection, seen from the daemon.
#[derive(Debug)]
pub struct Connection {
    framed: Framed<Stream>,
    peer: Option<PeerCred>,
    client_version: Option<String>,
}

impl Connection {
    fn new(stream: Stream, peer: Option<PeerCred>) -> Self {
        Self {
            framed: Framed::new(stream),
            peer,
            client_version: None,
        }
    }

    /// The client's credentials, for authorization. `None` on Windows.
    pub fn peer(&self) -> Option<PeerCred> {
        self.peer
    }

    /// The next request after the handshake, or `None` when the client hangs up.
    ///
    /// `Hello` is answered here; a version mismatch is reported to the client and returned.
    pub async fn next_request(&mut self) -> Result<Option<Request>, IpcError> {
        loop {
            let request = match self.framed.recv::<Request>().await {
                Ok(Some(request)) => request,
                Ok(None) => return Ok(None),
                Err(error @ IpcError::VersionMismatch { .. }) => {
                    self.reject(ErrorCode::VersionMismatch, error.to_string())
                        .await;
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            #[expect(clippy::wildcard_enum_match_arm, reason = "only Hello is handled here")]
            match request {
                Request::Hello {
                    protocol,
                    client_version,
                } => {
                    if protocol != PROTOCOL_VERSION {
                        let error = IpcError::VersionMismatch {
                            ours: PROTOCOL_VERSION,
                            theirs: protocol,
                        };
                        self.reject(ErrorCode::VersionMismatch, error.to_string())
                            .await;
                        return Err(error);
                    }
                    self.client_version = Some(client_version);
                    self.respond(Response::Hello {
                        protocol: PROTOCOL_VERSION,
                        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
                    })
                    .await?;
                }
                other if self.client_version.is_some() => return Ok(Some(other)),
                _ => {
                    self.reject(ErrorCode::HelloRequired, "send Hello first".to_owned())
                        .await;
                    return Err(IpcError::Unexpected);
                }
            }
        }
    }

    /// Sends the answer to the last request.
    pub async fn respond(&mut self, response: Response) -> Result<(), IpcError> {
        self.framed.send(&ServerMessage::Response(response)).await
    }

    /// Pushes an event to a client that sent [`Request::Watch`].
    pub async fn send_event(&mut self, event: Event) -> Result<(), IpcError> {
        self.framed.send(&ServerMessage::Event(event)).await
    }

    async fn reject(&mut self, code: ErrorCode, message: String) {
        // The connection is being dropped; failing to explain why changes nothing.
        let _ignored = self.respond(Response::Error { code, message }).await;
    }
}
