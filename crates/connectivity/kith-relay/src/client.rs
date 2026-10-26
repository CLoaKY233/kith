use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use kith_crypto::{SigningKey, fill_random};
use kith_relay_proto::{Decoder, Event, MAX_PAYLOAD_LEN, RejectReason, RelayClient};
use kith_types::NodeId;
use rustls_pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_rustls::TlsConnector;
use tracing::{debug, warn};

use crate::{Ended, Error, READ_CHUNK, tls, write_flushed};

/// Datagrams received through the relay, with their authenticated senders.
pub type Incoming = mpsc::Receiver<(NodeId, Vec<u8>)>;

/// How the client authenticates the relay's TLS certificate.
#[derive(Debug, Clone)]
pub enum Verification {
    /// Standard `WebPKI` validation against the bundled Mozilla roots.
    WebPki,
    /// Accept exactly this end-entity certificate.
    Pinned(CertificateDer<'static>),
}

/// Client timing and queue sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientOptions {
    /// Ping interval while connected, keeping the server's idle timer from firing.
    pub keepalive: Duration,
    /// The connection is dropped if the relay sends nothing for this long.
    pub idle_timeout: Duration,
    /// First redial delay; doubles on each failure.
    pub min_backoff: Duration,
    /// Upper bound on the redial delay.
    pub max_backoff: Duration,
    /// Datagrams queued in each direction before new ones are dropped.
    pub queue_len: usize,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            keepalive: Duration::from_secs(20),
            idle_timeout: Duration::from_secs(60),
            min_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(30),
            queue_len: 1024,
        }
    }
}

/// The client's connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Dialing or authenticating.
    Connecting,
    /// Authenticated; datagrams flow.
    Connected,
    /// The relay refused us. Final: the client stops redialing.
    Rejected(RejectReason),
}

/// [`Client::send`] did not queue the datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SendError {
    /// The datagram exceeds the relay's payload limit.
    #[error("datagram of {0} bytes exceeds the relay limit")]
    TooLarge(usize),
    /// The outgoing queue is full; the datagram was dropped.
    #[error("relay send queue full; datagram dropped")]
    Full,
    /// The client was rejected and has stopped.
    #[error("relay client is closed")]
    Closed,
}

/// A relay client. Dropping it closes the connection.
#[derive(Debug)]
pub struct Client {
    outgoing: mpsc::Sender<(NodeId, Vec<u8>)>,
    status: watch::Receiver<Status>,
    task: JoinHandle<()>,
}

impl Client {
    /// Connects over TLS to `address` (`host:port`), verifying the certificate for `server_name`.
    pub fn tls(
        address: String,
        server_name: &str,
        verification: Verification,
        relay: NodeId,
        key: Arc<SigningKey>,
        options: ClientOptions,
    ) -> Result<(Self, Incoming), Error> {
        let name =
            ServerName::try_from(server_name.to_owned()).map_err(|_| Error::InvalidServerName)?;
        let connector = TlsConnector::from(Arc::new(tls::client_config(verification)?));
        let dial = move || {
            let (address, name, connector) = (address.clone(), name.clone(), connector.clone());
            async move { connector.connect(name, dial_tcp(&address).await?).await }
        };
        Ok(Self::with_dialer(dial, relay, key, options))
    }

    /// Connects over plain TCP to `address` (`host:port`). For local testing.
    pub fn tcp(
        address: String,
        relay: NodeId,
        key: Arc<SigningKey>,
        options: ClientOptions,
    ) -> (Self, Incoming) {
        let dial = move || {
            let address = address.clone();
            async move { dial_tcp(&address).await }
        };
        Self::with_dialer(dial, relay, key, options)
    }

    /// Connects through any byte stream; `dial` is called for each (re)connection.
    pub fn with_dialer<D, F, S>(
        dial: D,
        relay: NodeId,
        key: Arc<SigningKey>,
        options: ClientOptions,
    ) -> (Self, Incoming)
    where
        D: FnMut() -> F + Send + 'static,
        F: Future<Output = io::Result<S>> + Send + 'static,
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let queue_len = options.queue_len.max(1);
        let (outgoing, outgoing_rx) = mpsc::channel(queue_len);
        let (incoming, incoming_rx) = mpsc::channel(queue_len);
        let (status_tx, status) = watch::channel(Status::Connecting);
        let worker = Worker {
            key,
            relay,
            options,
            outgoing: outgoing_rx,
            incoming,
            status: status_tx,
        };
        let task = tokio::spawn(worker.run(dial));
        (
            Self {
                outgoing,
                status,
                task,
            },
            incoming_rx,
        )
    }

    /// Queues `datagram` for `to`, holding it across reconnects up to the queue length.
    pub fn send(&self, to: NodeId, datagram: Vec<u8>) -> Result<(), SendError> {
        if datagram.len() > MAX_PAYLOAD_LEN {
            return Err(SendError::TooLarge(datagram.len()));
        }
        self.outgoing
            .try_send((to, datagram))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SendError::Full,
                mpsc::error::TrySendError::Closed(_) => SendError::Closed,
            })
    }

    /// The current connection state.
    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    /// A receiver that observes every state change.
    pub fn watch_status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn dial_tcp(address: &str) -> io::Result<TcpStream> {
    let stream = kith_transport_udp::connect_tcp(address).await?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

struct Worker {
    key: Arc<SigningKey>,
    relay: NodeId,
    options: ClientOptions,
    outgoing: mpsc::Receiver<(NodeId, Vec<u8>)>,
    incoming: mpsc::Sender<(NodeId, Vec<u8>)>,
    status: watch::Sender<Status>,
}

impl Worker {
    async fn run<D, F, S>(mut self, mut dial: D)
    where
        D: FnMut() -> F,
        F: Future<Output = io::Result<S>>,
        S: AsyncRead + AsyncWrite,
    {
        let mut backoff = self.options.min_backoff;
        loop {
            let ended = match timeout(self.options.idle_timeout, dial()).await {
                Ok(Ok(stream)) => self.drive(stream).await,
                Ok(Err(error)) => Ended::Io(error),
                Err(_) => Ended::IdleTimeout,
            };
            let was_connected = *self.status.borrow() == Status::Connected;
            match ended {
                Ended::Rejected(reason) => {
                    warn!(relay = %self.relay, %ended, "relay refused this node");
                    self.status.send_replace(Status::Rejected(reason));
                    return;
                }
                Ended::Stopped => return,
                Ended::Malformed(_) | Ended::Unencodable(_) | Ended::Misbehaved(_) => {
                    warn!(relay = %self.relay, %ended, "relay connection failed");
                }
                Ended::PeerClosed | Ended::IdleTimeout | Ended::Io(_) | Ended::Closed(_) => {
                    debug!(relay = %self.relay, was_connected, %ended, "relay connection lost");
                }
            }
            if was_connected {
                self.status.send_replace(Status::Connecting);
                backoff = self.options.min_backoff;
            }
            sleep(jittered(backoff)).await;
            backoff = backoff.saturating_mul(2).min(self.options.max_backoff);
        }
    }

    async fn drive<S: AsyncRead + AsyncWrite>(&mut self, stream: S) -> Ended {
        let mut client = RelayClient::new(Arc::clone(&self.key), self.relay);
        let (mut reader, mut writer) = tokio::io::split(stream);
        let mut decoder = Decoder::new();
        let mut read_buffer = vec![0_u8; READ_CHUNK];
        let mut write_buffer = Vec::new();
        let mut deadline = Instant::now() + self.options.idle_timeout;
        let mut keepalive = tokio::time::interval(self.options.keepalive);
        let mut ping_counter = 0_u64;
        loop {
            tokio::select! {
                read = reader.read(&mut read_buffer) => {
                    let chunk = match read {
                        Ok(0) => return Ended::PeerClosed,
                        Ok(read) => read_buffer.get(..read).unwrap_or_default(),
                        Err(error) => return Ended::Io(error),
                    };
                    deadline = Instant::now() + self.options.idle_timeout;
                    decoder.push(chunk);
                    if let Err(ended) = feed(&mut decoder, &mut client) {
                        return ended;
                    }
                }
                datagram = self.outgoing.recv() => {
                    let Some((to, datagram)) = datagram else { return Ended::Stopped };
                    client.send(to, datagram).ok();
                    while let Ok((to, datagram)) = self.outgoing.try_recv() {
                        client.send(to, datagram).ok();
                    }
                }
                _ = keepalive.tick() => {
                    ping_counter = ping_counter.wrapping_add(1);
                    client.ping(ping_counter.to_be_bytes());
                }
                () = sleep_until(deadline) => return Ended::IdleTimeout,
            }
            while let Some(event) = client.poll_event() {
                match event {
                    Event::Connected => {
                        debug!(relay = %self.relay, "relay connected");
                        self.status.send_replace(Status::Connected);
                    }
                    Event::Rejected { reason } => return Ended::Rejected(reason),
                    Event::Packet { src, payload } => {
                        drop(self.incoming.try_send((src, payload)));
                    }
                    Event::PeerGone { .. } | Event::Pong { .. } | Event::Health { .. } => {}
                }
            }
            while let Some(frame) = client.poll_transmit() {
                if let Err(error) = frame.encode(&mut write_buffer) {
                    return Ended::Unencodable(error);
                }
            }
            if !write_buffer.is_empty()
                && let Err(ended) = write_flushed(&mut writer, &write_buffer).await
            {
                return ended;
            }
            write_buffer.clear();
        }
    }
}

fn feed(decoder: &mut Decoder, client: &mut RelayClient<Arc<SigningKey>>) -> Result<(), Ended> {
    while let Some(frame) = decoder.next_frame().map_err(Ended::Malformed)? {
        client.handle_frame(frame).map_err(Ended::Misbehaved)?;
    }
    Ok(())
}

fn jittered(base: Duration) -> Duration {
    let mut random = [0_u8; 4];
    if fill_random(&mut random).is_err() {
        return base;
    }
    let fraction = f64::from(u32::from_be_bytes(random)) / f64::from(u32::MAX);
    base.mul_f64(0.5 + fraction / 2.0)
}
