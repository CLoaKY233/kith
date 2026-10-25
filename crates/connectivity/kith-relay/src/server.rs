use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use kith_crypto::fill_random;
use kith_relay_proto::{
    Action, Authorizer, CloseReason, ConnId, Decoder, Frame, NONCE_LEN, RelayServer,
};
use kith_types::NodeId;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, warn};

use crate::{Ended, Error, READ_CHUNK, tls, write_flushed};

const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

/// Server limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerOptions {
    /// Connections served at once; further ones are closed on arrival.
    pub max_connections: usize,
    /// A connection that sends nothing for this long (including the TLS handshake) is closed.
    pub idle_timeout: Duration,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            max_connections: 4096,
            idle_timeout: Duration::from_secs(60),
        }
    }
}

/// The relay's TLS certificate chain and private key.
#[derive(Debug)]
pub struct TlsIdentity {
    /// End-entity certificate first.
    pub chain: Vec<CertificateDer<'static>>,
    /// The end-entity certificate's private key.
    pub key: PrivateKeyDer<'static>,
}

enum CoreEvent {
    Connect {
        conn: ConnId,
        commands: mpsc::UnboundedSender<Command>,
    },
    Frame {
        conn: ConnId,
        frame: Frame,
    },
    Transmitted {
        conn: ConnId,
        bytes: usize,
    },
    Disconnect {
        conn: ConnId,
    },
    RefreshMembership,
}

enum Command {
    Send(Frame),
    Close(CloseReason),
}

#[derive(Debug, Clone)]
struct Shared {
    core: mpsc::Sender<CoreEvent>,
    permits: Arc<Semaphore>,
    next_conn: Arc<AtomicU64>,
    idle_timeout: Duration,
}

/// A running relay server. Dropping it stops the relay and closes every connection.
#[derive(Debug)]
pub struct Server {
    shared: Shared,
    core: JoinHandle<()>,
    listener: Option<(SocketAddr, JoinHandle<()>)>,
}

impl Server {
    /// A relay with identity `identity` that serves streams passed to [`serve`](Self::serve).
    pub fn new<A>(identity: NodeId, authorizer: A, options: ServerOptions) -> Self
    where
        A: Authorizer + Send + 'static,
    {
        let (core, events) = mpsc::channel(1024);
        let core_task = tokio::spawn(run_core(RelayServer::new(identity, authorizer), events));
        Self {
            shared: Shared {
                core,
                permits: Arc::new(Semaphore::new(options.max_connections)),
                next_conn: Arc::new(AtomicU64::new(0)),
                idle_timeout: options.idle_timeout,
            },
            core: core_task,
            listener: None,
        }
    }

    /// A relay listening on `addr`, over TLS when `tls` is given and plain TCP otherwise.
    pub async fn bind<A>(
        addr: SocketAddr,
        identity: NodeId,
        authorizer: A,
        tls: Option<TlsIdentity>,
        options: ServerOptions,
    ) -> Result<Self, Error>
    where
        A: Authorizer + Send + 'static,
    {
        let acceptor = tls
            .map(|identity| tls::server_config(identity).map(|c| TlsAcceptor::from(Arc::new(c))))
            .transpose()?;
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let mut server = Self::new(identity, authorizer, options);
        let accept = tokio::spawn(accept_loop(listener, acceptor, server.shared.clone()));
        server.listener = Some((local, accept));
        Ok(server)
    }

    /// The bound address, when created with [`bind`](Self::bind).
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().map(|(addr, _)| *addr)
    }

    /// Serves one already-established stream until it closes. Counts toward the limit.
    pub async fn serve<S>(&self, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        if let Ok(permit) = Arc::clone(&self.shared.permits).try_acquire_owned() {
            serve_connection(stream, None, &self.shared).await;
            drop(permit);
        } else {
            debug!("relay at its connection limit; refusing a stream");
        }
    }

    /// Closes connections of nodes the authorizer no longer accepts.
    pub async fn refresh_membership(&self) {
        drop(self.shared.core.send(CoreEvent::RefreshMembership).await);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some((_, accept)) = &self.listener {
            accept.abort();
        }
        self.core.abort();
    }
}

async fn accept_loop(listener: TcpListener, acceptor: Option<TlsAcceptor>, shared: Shared) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
                ) =>
            {
                continue;
            }
            Err(error) => {
                warn!(%error, "relay accept failed");
                sleep(ACCEPT_BACKOFF).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&shared.permits).try_acquire_owned() else {
            debug!(%peer, "relay at its connection limit; refusing a connection");
            continue;
        };
        drop(stream.set_nodelay(true));
        let shared = shared.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            match acceptor {
                Some(acceptor) => match timeout(shared.idle_timeout, acceptor.accept(stream)).await
                {
                    Ok(Ok(stream)) => serve_connection(stream, Some(peer), &shared).await,
                    Ok(Err(error)) => debug!(%peer, %error, "relay TLS handshake failed"),
                    Err(_) => debug!(%peer, "relay TLS handshake timed out"),
                },
                None => serve_connection(stream, Some(peer), &shared).await,
            }
            drop(permit);
        });
    }
}

async fn serve_connection<S>(stream: S, peer: Option<SocketAddr>, shared: &Shared)
where
    S: AsyncRead + AsyncWrite,
{
    let conn = ConnId(shared.next_conn.fetch_add(1, Ordering::Relaxed));
    let (commands, command_rx) = mpsc::unbounded_channel();
    if shared
        .core
        .send(CoreEvent::Connect { conn, commands })
        .await
        .is_err()
    {
        return;
    }
    debug!(conn = conn.0, ?peer, "relay connection accepted");
    let ended = pump(stream, conn, command_rx, shared).await;
    if let Ended::Unencodable(_) = ended {
        warn!(conn = conn.0, %ended, "relay connection failed");
    } else {
        debug!(conn = conn.0, %ended, "relay connection closed");
    }
    drop(shared.core.send(CoreEvent::Disconnect { conn }).await);
}

async fn pump<S: AsyncRead + AsyncWrite>(
    stream: S,
    conn: ConnId,
    mut commands: mpsc::UnboundedReceiver<Command>,
    shared: &Shared,
) -> Ended {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut decoder = Decoder::new();
    let mut read_buffer = vec![0_u8; READ_CHUNK];
    let mut write_buffer = Vec::new();
    let mut deadline = Instant::now() + shared.idle_timeout;
    loop {
        tokio::select! {
            read = reader.read(&mut read_buffer) => {
                let chunk = match read {
                    Ok(0) => return Ended::PeerClosed,
                    Ok(read) => read_buffer.get(..read).unwrap_or_default(),
                    Err(error) => return Ended::Io(error),
                };
                deadline = Instant::now() + shared.idle_timeout;
                decoder.push(chunk);
                if let Err(ended) = forward_frames(&mut decoder, conn, &shared.core).await {
                    return ended;
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ended::Stopped };
                let mut closing = None;
                let mut next = Some(command);
                while let Some(command) = next {
                    match command {
                        Command::Send(frame) => {
                            if let Err(error) = frame.encode(&mut write_buffer) {
                                closing = Some(Ended::Unencodable(error));
                            }
                        }
                        Command::Close(reason) => closing = Some(Ended::Closed(reason)),
                    }
                    next = if closing.is_some() { None } else { commands.try_recv().ok() };
                }
                if let Err(ended) = write_flushed(&mut writer, &write_buffer).await {
                    return ended;
                }
                let bytes = write_buffer.len();
                write_buffer.clear();
                drop(shared.core.send(CoreEvent::Transmitted { conn, bytes }).await);
                if let Some(ended) = closing {
                    drop(writer.shutdown().await);
                    return ended;
                }
            }
            () = sleep_until(deadline) => return Ended::IdleTimeout,
        }
    }
}

async fn forward_frames(
    decoder: &mut Decoder,
    conn: ConnId,
    core: &mpsc::Sender<CoreEvent>,
) -> Result<(), Ended> {
    while let Some(frame) = decoder.next_frame().map_err(Ended::Malformed)? {
        core.send(CoreEvent::Frame { conn, frame })
            .await
            .map_err(|_| Ended::Stopped)?;
    }
    Ok(())
}

async fn run_core<A: Authorizer>(
    mut server: RelayServer<A>,
    mut events: mpsc::Receiver<CoreEvent>,
) {
    let mut connections: BTreeMap<ConnId, mpsc::UnboundedSender<Command>> = BTreeMap::new();
    while let Some(event) = events.recv().await {
        match event {
            CoreEvent::Connect { conn, commands } => {
                let mut nonce = [0_u8; NONCE_LEN];
                if let Err(error) = fill_random(&mut nonce) {
                    warn!(conn = conn.0, %error, "relay refused a connection");
                    continue;
                }
                if server.on_connect(conn, nonce).is_ok() {
                    connections.insert(conn, commands);
                }
            }
            CoreEvent::Frame { conn, frame } => server.on_frame(conn, frame),
            CoreEvent::Transmitted { conn, bytes } => server.on_transmitted(conn, bytes),
            CoreEvent::Disconnect { conn } => {
                connections.remove(&conn);
                server.on_disconnect(conn);
            }
            CoreEvent::RefreshMembership => server.refresh_membership(),
        }
        while let Some(action) = server.poll_action() {
            match action {
                Action::Send { conn, frame } => {
                    if let Some(commands) = connections.get(&conn) {
                        drop(commands.send(Command::Send(frame)));
                    }
                }
                Action::Close { conn, reason } => {
                    if let Some(commands) = connections.remove(&conn) {
                        drop(commands.send(Command::Close(reason)));
                    }
                }
            }
        }
    }
}
