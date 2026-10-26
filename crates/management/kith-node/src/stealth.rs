use core::net::SocketAddr;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, PoisonError, RwLock};

use kith_config::Config;
use kith_crypto::SigningKey;
use kith_relay::{Client, ClientOptions, Incoming};
use kith_transport::{
    Action, AttemptId, FailureReason, Ladder, LadderConfig, LinkEvent, LinkId, TransportKind,
    TransportSet,
};
use kith_transport_quic::{QuicEndpoint, QuicError, QuicLink};
use kith_transport_ws::{WsLink, WsServer};
use kith_types::NodeId;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle, JoinSet};
use tokio::time::Instant;
use tracing::{debug, warn};

use crate::server::stun_reply;
use crate::types::{Command, ConnectivityEvent};

const DEFAULT_QUIC_PORT: u16 = 443;
const DEFAULT_SNI: &str = "www.example.com";
const UDP_ATTEMPT: Duration = Duration::from_secs(3);
const DIAL_ATTEMPT: Duration = Duration::from_secs(10);
const RELAY_DIAL_BUDGET: Duration = Duration::from_secs(30);
const WS_BUFFER: usize = 64 * 1024;
const STUN_BUFFER: usize = 1500;

/// Aborts the held tasks when dropped.
#[derive(Debug, Default)]
pub(crate) struct Tasks(pub(crate) Vec<JoinHandle<()>>);

impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

/// Which transports this node may use, from `[network.transports]` and `[stealth]`.
#[derive(Debug, Clone)]
pub(crate) struct StealthOptions {
    peer_kinds: TransportSet,
    relay_kinds: TransportSet,
    websocket_urls: Vec<String>,
    ws_server_name: Option<String>,
    quic_sni: Arc<str>,
    quic_port: u16,
    stealth_only: bool,
}

impl Default for StealthOptions {
    fn default() -> Self {
        Self {
            peer_kinds: TransportSet::EMPTY.with(TransportKind::Udp),
            relay_kinds: TransportSet::EMPTY.with(TransportKind::Udp),
            websocket_urls: Vec::new(),
            ws_server_name: None,
            quic_sni: DEFAULT_SNI.into(),
            quic_port: DEFAULT_QUIC_PORT,
            stealth_only: false,
        }
    }
}

impl StealthOptions {
    /// Reads the transport ladder settings from the node config.
    pub(crate) fn from_config(config: &Config) -> Self {
        let transports = &config.network.transports;
        let mut peer_kinds = TransportSet::EMPTY;
        let mut relay_kinds = TransportSet::EMPTY;
        for kind in &transports.order {
            match kind {
                kith_config::TransportKind::Udp if !transports.stealth_only => {
                    peer_kinds = peer_kinds.with(TransportKind::Udp);
                    relay_kinds = relay_kinds.with(TransportKind::Udp);
                }
                kith_config::TransportKind::Quic => {
                    peer_kinds = peer_kinds.with(TransportKind::Quic);
                }
                kith_config::TransportKind::Websocket
                    if !config.stealth.websocket_urls.is_empty() =>
                {
                    relay_kinds = relay_kinds.with(TransportKind::WebSocket);
                }
                kith_config::TransportKind::Udp | kith_config::TransportKind::Websocket => {}
            }
        }
        if relay_kinds.is_empty() {
            relay_kinds = relay_kinds.with(TransportKind::Udp);
        }
        Self {
            peer_kinds,
            relay_kinds,
            websocket_urls: config.stealth.websocket_urls.clone(),
            ws_server_name: config.stealth.cdn_front.clone(),
            quic_sni: config
                .stealth
                .quic_sni
                .as_deref()
                .or(config.server.public_hostname.as_deref())
                .unwrap_or(DEFAULT_SNI)
                .into(),
            quic_port: config
                .server
                .quic_listen
                .map_or(DEFAULT_QUIC_PORT, |addr| addr.port()),
            stealth_only: transports.stealth_only,
        }
    }

    /// Whether plain WireGuard UDP must never be sent.
    pub(crate) const fn stealth_only(&self) -> bool {
        self.stealth_only
    }

    /// Whether server-mode peers get a QUIC rung.
    pub(crate) const fn wants_quic(&self) -> bool {
        self.peer_kinds.contains(TransportKind::Quic)
    }
}

#[derive(Debug, Default)]
struct LinkTable {
    next: u64,
    links: BTreeMap<NodeId, (u64, QuicLink)>,
}

/// QUIC links that carry WireGuard datagrams, keyed by the authenticated peer.
#[derive(Debug, Clone, Default)]
pub(crate) struct QuicLinks(Arc<RwLock<LinkTable>>);

impl QuicLinks {
    fn insert(&self, link: QuicLink) -> u64 {
        let mut table = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let token = table.next;
        table.next = table.next.wrapping_add(1);
        let node = link.remote_node();
        if let Some((_, old)) = table.links.insert(node, (token, link)) {
            old.close();
        }
        token
    }

    fn remove(&self, node: NodeId, token: u64) {
        let mut table = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if table
            .links
            .get(&node)
            .is_some_and(|(held, _)| *held == token)
        {
            table.links.remove(&node);
        }
    }

    /// Sends a WireGuard datagram over the peer's QUIC link; false if there is none or it failed.
    pub(crate) fn send(&self, node: NodeId, datagram: &[u8]) -> bool {
        let table = self.0.read().unwrap_or_else(PoisonError::into_inner);
        table
            .links
            .get(&node)
            .is_some_and(|(_, link)| link.send(datagram).is_ok())
    }

    async fn serve(self, link: QuicLink, token: u64, commands: mpsc::Sender<Command>) {
        let from = link.remote_node();
        loop {
            match link.recv().await {
                Ok(datagram) => {
                    let event = ConnectivityEvent::Relayed {
                        from,
                        datagram: datagram.to_vec(),
                    };
                    if commands.send(Command::Connectivity(event)).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    debug!(peer = %from, %error, "QUIC link ended");
                    break;
                }
            }
        }
        self.remove(from, token);
    }
}

/// Binds the client-side QUIC endpoint used to dial server-mode peers.
pub(crate) fn client_endpoint(key: &SigningKey) -> Option<Arc<QuicEndpoint>> {
    let any = SocketAddr::from(([0, 0, 0, 0], 0));
    match QuicEndpoint::bind(any, &key.to_seed()) {
        Ok(endpoint) => Some(Arc::new(endpoint)),
        Err(error) => {
            warn!(%error, "could not bind the QUIC client endpoint");
            None
        }
    }
}

/// Accepts QUIC links from members and carries their WireGuard datagrams.
pub(crate) async fn accept_quic(
    endpoint: Arc<QuicEndpoint>,
    members: Arc<RwLock<BTreeSet<NodeId>>>,
    links: QuicLinks,
    commands: mpsc::Sender<Command>,
) {
    loop {
        match endpoint.accept().await {
            Ok((link, remote)) => {
                let member = members
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains(&remote);
                if member {
                    let token = links.insert(link.clone());
                    tokio::spawn(links.clone().serve(link, token, commands.clone()));
                } else {
                    debug!(peer = %remote, "refusing QUIC link from a non-member");
                    link.close();
                }
            }
            Err(QuicError::EndpointClosed) => return,
            Err(error) => debug!(%error, "QUIC handshake failed"),
        }
    }
}

/// Accepts WebSocket upgrades and serves each as a relay connection.
pub(crate) async fn accept_ws(server: WsServer, relay: Arc<kith_relay::Server>) {
    loop {
        match server.accept().await {
            Ok(incoming) => {
                let relay = Arc::clone(&relay);
                tokio::spawn(async move {
                    match incoming.upgrade().await {
                        Ok(Some(link)) => relay.serve(ws_stream(link)).await,
                        Ok(None) => {}
                        Err(error) => debug!(%error, "WebSocket upgrade failed"),
                    }
                });
            }
            Err(error) => {
                warn!(%error, "WebSocket listener failed");
                return;
            }
        }
    }
}

/// Answers STUN binding requests on a UDP socket.
pub(crate) async fn serve_stun(socket: UdpSocket) {
    let mut buffer = [0_u8; STUN_BUFFER];
    loop {
        let (len, source) = match socket.recv_from(&mut buffer).await {
            Ok(received) => received,
            Err(error) => {
                debug!(%error, "STUN socket receive failed");
                continue;
            }
        };
        if let Some(reply) = buffer
            .get(..len)
            .and_then(|datagram| stun_reply(source, datagram))
            && let Err(error) = socket.send_to(&reply, source).await
        {
            debug!(%error, "STUN reply failed");
        }
    }
}

fn ws_stream(link: WsLink) -> DuplexStream {
    let (outer, inner) = tokio::io::duplex(WS_BUFFER);
    tokio::spawn(pump_ws(link, inner));
    outer
}

async fn pump_ws(mut link: WsLink, inner: DuplexStream) {
    let sender = link.sender();
    let (mut reader, mut writer) = tokio::io::split(inner);
    let upstream = async {
        let mut buffer = vec![0_u8; WS_BUFFER];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(len) => {
                    let Some(chunk) = buffer.get(..len) else {
                        return;
                    };
                    if sender.send(chunk).await.is_err() {
                        return;
                    }
                }
            }
        }
    };
    let downstream = async {
        while let Ok(message) = link.recv().await {
            if writer.write_all(&message).await.is_err() {
                return;
            }
        }
    };
    tokio::select! {
        () = upstream => {}
        () = downstream => {}
    }
    drop(sender.close().await);
}

/// The byte stream a relay client runs over: plain TCP or a WebSocket tunnel.
#[derive(Debug)]
pub(crate) enum RelayStream {
    /// Plain TCP.
    Tcp(TcpStream),
    /// A WebSocket link bridged to a byte stream.
    Ws(DuplexStream),
}

impl AsyncRead for RelayStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            Self::Ws(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for RelayStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            Self::Ws(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            Self::Ws(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            Self::Ws(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

struct RelayDialer {
    address: String,
    urls: Vec<String>,
    ws: kith_transport_ws::ClientOptions,
    ladder: Ladder,
    epoch: Instant,
    last: Option<LinkId>,
    next_link: u64,
}

type RelayAttempt = (AttemptId, Duration, Result<RelayStream, FailureReason>);

impl RelayDialer {
    async fn dial(&mut self) -> io::Result<RelayStream> {
        if let Some(link) = self.last.take() {
            let now = self.epoch.elapsed();
            self.ladder.handle_event(
                now,
                LinkEvent::Down {
                    link,
                    reason: FailureReason::Closed,
                },
            );
        }
        let budget = tokio::time::sleep(RELAY_DIAL_BUDGET);
        tokio::pin!(budget);
        let mut attempts: JoinSet<RelayAttempt> = JoinSet::new();
        let mut pending = BTreeSet::new();
        let result = loop {
            let now = self.epoch.elapsed();
            while let Some(action) = self.ladder.poll_action(now) {
                if let Action::Start { kind, attempt } = action {
                    pending.insert(attempt);
                    attempts.spawn(self.attempt(kind, attempt, now));
                }
            }
            let deadline = self
                .ladder
                .poll_timeout()
                .and_then(|at| self.epoch.checked_add(at));
            tokio::select! {
                Some(Ok((attempt, started, outcome))) = attempts.join_next() => {
                    pending.remove(&attempt);
                    let now = self.epoch.elapsed();
                    match outcome {
                        Ok(stream) => {
                            let link = LinkId(self.next_link);
                            self.next_link = self.next_link.wrapping_add(1);
                            let rtt = now.saturating_sub(started);
                            self.ladder.handle_event(now, LinkEvent::Up { link, attempt, rtt });
                            self.last = Some(link);
                            break Ok(stream);
                        }
                        Err(reason) => self.ladder.handle_event(now, LinkEvent::Failed { attempt, reason }),
                    }
                }
                () = sleep_until(deadline) => {}
                () = &mut budget => break Err(io::Error::new(io::ErrorKind::TimedOut, "no relay transport connected")),
            }
        };
        let now = self.epoch.elapsed();
        for attempt in pending {
            self.ladder.handle_event(
                now,
                LinkEvent::Failed {
                    attempt,
                    reason: FailureReason::Other,
                },
            );
        }
        result
    }

    fn attempt(
        &self,
        kind: TransportKind,
        attempt: AttemptId,
        started: Duration,
    ) -> impl Future<Output = RelayAttempt> + Send + 'static {
        let address = self.address.clone();
        let urls = self.urls.clone();
        let options = self.ws.clone();
        async move {
            let outcome = match kind {
                TransportKind::Udp => dial_tcp(&address).await,
                TransportKind::WebSocket => dial_ws(&urls, &options).await,
                TransportKind::UdpObfuscated | TransportKind::Quic | TransportKind::Reality => {
                    Err(FailureReason::Other)
                }
            };
            (attempt, started, outcome)
        }
    }
}

async fn dial_tcp(address: &str) -> Result<RelayStream, FailureReason> {
    let stream = tokio::time::timeout(DIAL_ATTEMPT, kith_transport_udp::connect_tcp(address))
        .await
        .map_err(|_| FailureReason::Timeout)?
        .map_err(|error| {
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset
            ) {
                FailureReason::Refused
            } else {
                FailureReason::Other
            }
        })?;
    stream.set_nodelay(true).map_err(|_| FailureReason::Other)?;
    Ok(RelayStream::Tcp(stream))
}

async fn dial_ws(
    urls: &[String],
    options: &kith_transport_ws::ClientOptions,
) -> Result<RelayStream, FailureReason> {
    let mut reason = FailureReason::Other;
    for url in urls {
        match tokio::time::timeout(DIAL_ATTEMPT, kith_transport_ws::connect(url, options)).await {
            Ok(Ok(link)) => return Ok(RelayStream::Ws(ws_stream(link))),
            Ok(Err(error)) => {
                debug!(%url, %error, "WebSocket relay dial failed");
                reason = FailureReason::ProtocolError;
            }
            Err(_) => reason = FailureReason::Timeout,
        }
    }
    Err(reason)
}

/// Starts a relay client whose dials race plain TCP and WebSocket on a ladder.
pub(crate) fn relay_client(
    address: String,
    relay: NodeId,
    key: Arc<SigningKey>,
    options: &StealthOptions,
) -> (Client, Incoming) {
    let config = LadderConfig {
        enabled: options.relay_kinds,
        ..LadderConfig::default()
    };
    let ladder = match Ladder::new(config) {
        Ok(ladder) => ladder,
        Err(error) => {
            warn!(%error, "relay ladder config rejected; using plain TCP");
            return Client::tcp(address, relay, key, ClientOptions::default());
        }
    };
    let dialer = Arc::new(Mutex::new(RelayDialer {
        address,
        urls: options.websocket_urls.clone(),
        ws: kith_transport_ws::ClientOptions {
            server_name: options.ws_server_name.clone(),
            ..kith_transport_ws::ClientOptions::default()
        },
        ladder,
        epoch: Instant::now(),
        last: None,
        next_link: 0,
    }));
    let dial = move || {
        let dialer = Arc::clone(&dialer);
        async move { dialer.lock().await.dial().await }
    };
    Client::with_dialer(dial, relay, key, ClientOptions::default())
}

/// What a peer ladder dials with and delivers to.
#[derive(Debug, Clone)]
pub(crate) struct LadderIo {
    /// The client QUIC endpoint.
    pub(crate) endpoint: Arc<QuicEndpoint>,
    /// Where established QUIC links are registered.
    pub(crate) links: QuicLinks,
    /// Where received datagrams go.
    pub(crate) commands: mpsc::Sender<Command>,
}

/// Starts the transport ladder for a server-mode peer reachable at `address` (`host:port`).
pub(crate) fn spawn_peer_ladder(
    node: NodeId,
    address: &str,
    options: &StealthOptions,
    direct: watch::Receiver<bool>,
    io: LadderIo,
) -> Option<JoinHandle<()>> {
    let host = address
        .rsplit_once(':')
        .map_or(address, |(host, _)| host)
        .trim_start_matches('[')
        .trim_end_matches(']');
    let config = LadderConfig {
        enabled: options.peer_kinds.intersection(
            TransportSet::EMPTY
                .with(TransportKind::Udp)
                .with(TransportKind::Quic),
        ),
        ..LadderConfig::default()
    };
    let ladder = match Ladder::new(config) {
        Ok(ladder) => ladder,
        Err(error) => {
            debug!(peer = %node, %error, "no peer ladder");
            return None;
        }
    };
    let driver = PeerDriver {
        node,
        target: format!("{host}:{}", options.quic_port).into(),
        sni: Arc::clone(&options.quic_sni),
        endpoint: io.endpoint,
        direct,
        links: io.links,
        commands: io.commands,
    };
    Some(tokio::spawn(driver.run(ladder)))
}

struct OpenLink {
    token: u64,
    link: QuicLink,
    serving: AbortHandle,
}

type PeerAttempt = (AttemptId, Duration, Result<Option<QuicLink>, FailureReason>);

struct PeerDriver {
    node: NodeId,
    target: Arc<str>,
    sni: Arc<str>,
    endpoint: Arc<QuicEndpoint>,
    direct: watch::Receiver<bool>,
    links: QuicLinks,
    commands: mpsc::Sender<Command>,
}

impl PeerDriver {
    async fn run(mut self, mut ladder: Ladder) {
        let epoch = Instant::now();
        let mut attempts: JoinSet<PeerAttempt> = JoinSet::new();
        let mut serving: JoinSet<LinkId> = JoinSet::new();
        let mut udp: Option<LinkId> = None;
        let mut quic: BTreeMap<LinkId, OpenLink> = BTreeMap::new();
        let mut next_link = 0_u64;
        loop {
            let now = epoch.elapsed();
            while let Some(action) = ladder.poll_action(now) {
                match action {
                    Action::Start { kind, attempt } => {
                        attempts.spawn(self.attempt(kind, attempt, now));
                    }
                    Action::Close(link) => {
                        if udp == Some(link) {
                            udp = None;
                        }
                        if let Some(open) = quic.remove(&link) {
                            open.serving.abort();
                            self.links.remove(self.node, open.token);
                            open.link.close();
                        }
                    }
                }
            }
            let deadline = ladder.poll_timeout().and_then(|at| epoch.checked_add(at));
            tokio::select! {
                Some(Ok((attempt, started, outcome))) = attempts.join_next() => {
                    let now = epoch.elapsed();
                    match outcome {
                        Ok(link) => {
                            let id = LinkId(next_link);
                            next_link = next_link.wrapping_add(1);
                            match link {
                                None => udp = Some(id),
                                Some(link) => {
                                    let token = self.links.insert(link.clone());
                                    let links = self.links.clone();
                                    let commands = self.commands.clone();
                                    let reader = link.clone();
                                    let serving_handle = serving.spawn(async move {
                                        links.serve(reader, token, commands).await;
                                        id
                                    });
                                    quic.insert(id, OpenLink { token, link, serving: serving_handle });
                                }
                            }
                            let rtt = now.saturating_sub(started);
                            ladder.handle_event(now, LinkEvent::Up { link: id, attempt, rtt });
                        }
                        Err(reason) => ladder.handle_event(now, LinkEvent::Failed { attempt, reason }),
                    }
                }
                Some(Ok(id)) = serving.join_next() => {
                    if quic.remove(&id).is_some() {
                        ladder.handle_event(epoch.elapsed(), LinkEvent::Down { link: id, reason: FailureReason::Closed });
                    }
                }
                changed = self.direct.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let up = *self.direct.borrow_and_update();
                    if !up && let Some(id) = udp.take() {
                        ladder.handle_event(epoch.elapsed(), LinkEvent::Down { link: id, reason: FailureReason::NetworkChanged });
                    }
                }
                () = sleep_until(deadline) => {}
            }
        }
        for open in quic.into_values() {
            self.links.remove(self.node, open.token);
            open.link.close();
        }
    }

    fn attempt(
        &self,
        kind: TransportKind,
        attempt: AttemptId,
        started: Duration,
    ) -> impl Future<Output = PeerAttempt> + Send + 'static {
        let mut direct = self.direct.clone();
        let endpoint = Arc::clone(&self.endpoint);
        let target = Arc::clone(&self.target);
        let sni = Arc::clone(&self.sni);
        let node = self.node;
        async move {
            let outcome = match kind {
                TransportKind::Udp => {
                    match tokio::time::timeout(UDP_ATTEMPT, direct.wait_for(|up| *up)).await {
                        Ok(Ok(_)) => Ok(None),
                        Ok(Err(_)) => Err(FailureReason::Other),
                        Err(_) => Err(FailureReason::Timeout),
                    }
                }
                TransportKind::Quic => dial_quic(&endpoint, &target, node, &sni).await.map(Some),
                TransportKind::UdpObfuscated
                | TransportKind::WebSocket
                | TransportKind::Reality => Err(FailureReason::Other),
            };
            (attempt, started, outcome)
        }
    }
}

async fn dial_quic(
    endpoint: &QuicEndpoint,
    target: &str,
    node: NodeId,
    sni: &str,
) -> Result<QuicLink, FailureReason> {
    let dial = async {
        let addr = tokio::net::lookup_host(target)
            .await
            .map_err(|_| FailureReason::Other)?
            .find(SocketAddr::is_ipv4)
            .ok_or(FailureReason::Other)?;
        endpoint.connect(addr, node, sni).await.map_err(|error| {
            debug!(peer = %node, %error, "QUIC dial failed");
            FailureReason::ProtocolError
        })
    };
    let link = tokio::time::timeout(DIAL_ATTEMPT, dial)
        .await
        .map_err(|_| FailureReason::Timeout)??;
    if link.max_datagram_size().is_none() {
        link.close();
        return Err(FailureReason::ProtocolError);
    }
    Ok(link)
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => core::future::pending().await,
    }
}
