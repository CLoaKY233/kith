use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use kith_crypto::SigningKey;
use kith_ledger::{JoinProof, Ledger, SignedOp};
use kith_sync::SyncSession;
use kith_types::{NetworkId, NodeId};
use rustls::pki_types::ServerName;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};
use tracing::{debug, warn};

use crate::connection::{self, Command, Peer, shared_networks};
use crate::message::{FrameReader, Message, Presence, write_message};
use crate::{ChannelError, ChannelHandler};

pub(crate) const ALPN: &[u8] = b"kith-control/1";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

const JOIN_READ_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_INBOUND: usize = 1024;

const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// The latest frame of each kind of gossip, per network.
type GossipFrames = BTreeMap<(NetworkId, Topic), Arc<[u8]>>;

/// A kind of gossip; each network keeps the latest of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Topic {
    Endpoints,
    Presence,
}

pub(crate) struct Hub<H> {
    pub(crate) handler: Arc<H>,
    identity: SigningKey,
    acceptor: TlsAcceptor,
    connections: Mutex<BTreeMap<u64, Registered>>,
    /// The latest gossip of each kind per network, sent to members as they connect.
    gossip: Mutex<GossipFrames>,
    next_id: AtomicU64,
}

struct Registered {
    peer: NodeId,
    addr: SocketAddr,
    commands: mpsc::UnboundedSender<Command>,
}

impl<H> Hub<H> {
    pub(crate) fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn register(&self, peer: &Peer, commands: mpsc::UnboundedSender<Command>) {
        lock(&self.connections).insert(
            peer.id,
            Registered {
                peer: peer.node,
                addr: peer.addr,
                commands,
            },
        );
    }

    pub(crate) fn deregister(&self, id: u64) {
        lock(&self.connections).remove(&id);
    }

    pub(crate) fn broadcast(&self, command: &Command, except: Option<u64>) {
        for (id, connection) in lock(&self.connections).iter() {
            if Some(*id) != except {
                let _closing = connection.commands.send(command.clone());
            }
        }
    }

    pub(crate) fn latest_gossip(&self, network: NetworkId) -> Vec<Arc<[u8]>> {
        lock(&self.gossip)
            .iter()
            .filter(|((id, _), _)| *id == network)
            .map(|(_, frame)| Arc::clone(frame))
            .collect()
    }

    fn gossip(
        &self,
        network: NetworkId,
        topic: Topic,
        message: &Message,
    ) -> Result<(), ChannelError> {
        let frame: Arc<[u8]> = message.to_frame()?.into();
        lock(&self.gossip).insert((network, topic), Arc::clone(&frame));
        self.broadcast(&Command::Gossip(network, frame), None);
        Ok(())
    }

    async fn dial(
        &self,
        addr: SocketAddr,
        expected: NodeId,
    ) -> Result<TlsStream<TcpStream>, ChannelError> {
        let config = kith_tls::client_config(&self.identity, expected, &[ALPN])?;
        let tcp = timeout(CONNECT_TIMEOUT, kith_transport_udp::connect_tcp(addr))
            .await
            .map_err(|_| ChannelError::TimedOut)??;
        tcp.set_nodelay(true)?;
        let handshake = TlsConnector::from(config).connect(ServerName::from(addr.ip()), tcp);
        let stream = timeout(HANDSHAKE_TIMEOUT, handshake)
            .await
            .map_err(|_| ChannelError::TimedOut)??;
        Ok(stream.into())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A node's control channel, shared by its connections; dropping the last clone closes them.
pub struct Channel<H> {
    hub: Arc<Hub<H>>,
}

impl<H> Clone for Channel<H> {
    fn clone(&self) -> Self {
        Self {
            hub: Arc::clone(&self.hub),
        }
    }
}

impl<H> fmt::Debug for Channel<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("node", &self.hub.identity.node_id())
            .finish_non_exhaustive()
    }
}

impl<H: ChannelHandler> Channel<H> {
    /// A channel that authenticates as `identity` and reports to `handler`.
    pub fn new(identity: &SigningKey, handler: Arc<H>) -> Result<Self, ChannelError> {
        let acceptor = TlsAcceptor::from(kith_tls::server_config(identity, &[ALPN])?);
        Ok(Self {
            hub: Arc::new(Hub {
                handler,
                identity: SigningKey::from_seed(&identity.to_seed()),
                acceptor,
                connections: Mutex::new(BTreeMap::new()),
                gossip: Mutex::new(BTreeMap::new()),
                next_id: AtomicU64::new(0),
            }),
        })
    }

    /// Re-runs sync of `network` with every connected peer, after a local ledger change.
    pub fn notify_changed(&self, network: NetworkId) {
        self.hub.broadcast(&Command::Changed(network), None);
    }

    /// Gossips `entries` to the members of `network`, now and as they connect, until replaced.
    pub fn send_endpoints(
        &self,
        network: NetworkId,
        entries: Vec<(NodeId, Vec<SocketAddr>)>,
    ) -> Result<(), ChannelError> {
        self.hub.gossip(
            network,
            Topic::Endpoints,
            &Message::Endpoints { network, entries },
        )
    }

    /// Gossips which of the plain WireGuard members this node is the gateway for are
    /// online, now and as members connect, until replaced.
    pub fn send_presence(
        &self,
        network: NetworkId,
        entries: Vec<(NodeId, Presence)>,
    ) -> Result<(), ChannelError> {
        self.hub.gossip(
            network,
            Topic::Presence,
            &Message::Presence { network, entries },
        )
    }

    /// Connected members and the address of each connection.
    pub fn peers(&self) -> Vec<(NodeId, SocketAddr)> {
        lock(&self.hub.connections)
            .values()
            .map(|connection| (connection.peer, connection.addr))
            .collect()
    }
}

/// Accepts control connections until dropped; accepted ones close with the [`Channel`].
#[derive(Debug)]
pub struct ChannelServer {
    local_addr: SocketAddr,
    accept: JoinHandle<()>,
}

impl ChannelServer {
    /// Listens on `addr` and serves every accepted connection through `channel`.
    pub async fn bind<H: ChannelHandler>(
        channel: &Channel<H>,
        addr: SocketAddr,
    ) -> Result<Self, ChannelError> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let accept = tokio::spawn(accept_loop(listener, Arc::downgrade(&channel.hub)));
        Ok(Self { local_addr, accept })
    }

    /// The address the listener is bound to.
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl Drop for ChannelServer {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

async fn accept_loop<H: ChannelHandler>(listener: TcpListener, hub: Weak<Hub<H>>) {
    let slots = Arc::new(Semaphore::new(MAX_INBOUND));
    loop {
        let (tcp, addr) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                warn!(%error, "control channel accept failed");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
                continue;
            }
        };
        let Some(strong) = hub.upgrade() else {
            return;
        };
        let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
            debug!(%addr, "too many control connections, refusing one");
            continue;
        };
        let acceptor = strong.acceptor.clone();
        let handler = Arc::clone(&strong.handler);
        let id = strong.next_id();
        let weak = Weak::clone(&hub);
        tokio::spawn(async move {
            let _slot = slot;
            match accept_one(&acceptor, tcp).await {
                Ok((stream, node)) => {
                    let peer = Peer { id, node, addr };
                    connection::serve_accepted(weak, handler, peer, stream).await;
                }
                Err(error) => debug!(%addr, %error, "control handshake failed"),
            }
        });
    }
}

async fn accept_one(
    acceptor: &TlsAcceptor,
    tcp: TcpStream,
) -> Result<(TlsStream<TcpStream>, NodeId), ChannelError> {
    tcp.set_nodelay(true)?;
    let stream: TlsStream<TcpStream> = timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp))
        .await
        .map_err(|_| ChannelError::TimedOut)??
        .into();
    let node = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(kith_tls::peer_node_id)
        .ok_or(ChannelError::NotMember)?;
    Ok((stream, node))
}

/// One dialed connection to a member. Dropping it closes the connection.
#[derive(Debug)]
pub struct ChannelClient {
    keep: oneshot::Sender<()>,
}

impl ChannelClient {
    /// Dials the member `expected` at `addr` and keeps syncing every network both share.
    pub async fn connect<H: ChannelHandler>(
        channel: &Channel<H>,
        addr: SocketAddr,
        expected: NodeId,
    ) -> Result<Self, ChannelError> {
        let hub = &channel.hub;
        let offered = shared_networks(&*hub.handler, expected);
        if offered.is_empty() {
            return Err(ChannelError::NotMember);
        }
        let stream = hub.dial(addr, expected).await?;
        let peer = Peer {
            id: hub.next_id(),
            node: expected,
            addr,
        };
        let keep = connection::spawn_dialed(hub, peer, stream, offered);
        Ok(Self { keep })
    }

    /// Asks `sponsor` at `addr` to admit this device to `network`, and returns the synced ledger.
    pub async fn join<H: ChannelHandler>(
        channel: &Channel<H>,
        addr: SocketAddr,
        sponsor: NodeId,
        network: NetworkId,
        proof: JoinProof,
    ) -> Result<Ledger, ChannelError> {
        let hub = &channel.hub;
        let mut stream = hub.dial(addr, sponsor).await?;
        let mut frames = FrameReader::new();
        let request = Message::JoinRequest {
            network,
            proof: Box::new(proof),
        };
        write_message(&mut stream, &request).await?;
        let genesis = match read(&mut frames, &mut stream).await? {
            Message::JoinAccepted { genesis } => genesis,
            Message::JoinRejected { reason } => return Err(ChannelError::JoinRejected(reason)),
            Message::Hello { .. }
            | Message::JoinRequest { .. }
            | Message::Sync { .. }
            | Message::Endpoints { .. }
            | Message::Presence { .. } => return Err(ChannelError::UnexpectedMessage),
        };
        let mut ledger = Ledger::new(SignedOp::from_bytes(&genesis)?)?;
        if ledger.network_id() != network {
            return Err(ChannelError::UnexpectedMessage);
        }
        let mut session = SyncSession::new(false, network);
        while !session.is_done() {
            match read(&mut frames, &mut stream).await? {
                Message::Sync {
                    network: synced,
                    bytes,
                } if synced == network => {
                    session.handle_message(&mut ledger, &bytes)?;
                    while let Some(bytes) = session.poll_transmit(&ledger) {
                        write_message(&mut stream, &Message::Sync { network, bytes }).await?;
                    }
                }
                Message::Hello { .. } | Message::Endpoints { .. } | Message::Presence { .. } => {}
                Message::Sync { .. }
                | Message::JoinRequest { .. }
                | Message::JoinAccepted { .. }
                | Message::JoinRejected { .. } => return Err(ChannelError::UnexpectedMessage),
            }
        }
        if let Err(error) = stream.shutdown().await {
            debug!(%error, "closing the join connection failed");
        }
        if !ledger.view().is_member(&hub.identity.node_id()) {
            return Err(ChannelError::NotMember);
        }
        debug!(%network, ops = ledger.len(), "joined network");
        Ok(ledger)
    }

    /// Resolves once the connection has closed, so the caller can redial.
    pub async fn closed(&mut self) {
        self.keep.closed().await;
    }
}

#[cfg(test)]
impl<H> Channel<H> {
    pub(crate) async fn dial(
        &self,
        addr: SocketAddr,
        expected: NodeId,
    ) -> Result<TlsStream<TcpStream>, ChannelError> {
        self.hub.dial(addr, expected).await
    }
}

async fn read(
    frames: &mut FrameReader,
    stream: &mut TlsStream<TcpStream>,
) -> Result<Message, ChannelError> {
    timeout(JOIN_READ_TIMEOUT, frames.next(stream))
        .await
        .map_err(|_| ChannelError::TimedOut)?
}
