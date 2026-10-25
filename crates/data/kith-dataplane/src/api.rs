//! The dataplane's public face: configuration, commands and events.

use std::net::SocketAddr;
use std::time::Instant;

use kith_crypto::SigningKey;
use kith_policy::CompiledFilter;
use kith_router::{ExitError, MeshRanges, SetNetworkError};
use kith_transport_udp::{UdpError, UdpLink};
use kith_tun::{BatchError, Tun, TunError};
use kith_types::{IpPrefix, NetworkId, NodeId};
use kith_wg::WgError;
use tokio::sync::{mpsc, oneshot};

use crate::engine::Engine;

const COMMAND_QUEUE: usize = 64;
const EVENT_QUEUE: usize = 1024;

/// What [`Dataplane::new`] needs.
#[derive(Debug)]
pub struct DataplaneConfig {
    /// The device identity; its X25519 form is the WireGuard key.
    pub signing_key: SigningKey,
    /// The TUN device carrying every network.
    pub tun: Tun,
    /// The UDP socket shared by WireGuard, STUN and disco.
    pub link: UdpLink,
}

/// Where a peer's datagrams go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    /// Straight to this address over UDP.
    Direct(SocketAddr),
    /// Through the relay, emitted as [`Event::Relay`].
    Relay,
}

/// One WireGuard peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSpec {
    /// The network the peer belongs to; selects the inbound filter.
    pub network: NetworkId,
    /// The peer's identity, from which its WireGuard key is derived.
    pub node: NodeId,
    /// Source prefixes accepted from the peer.
    pub allowed_ips: Vec<IpPrefix>,
    /// Where to send; `None` keeps any endpoint already learned.
    pub endpoint: Option<Endpoint>,
    /// Persistent keepalive interval in seconds.
    pub persistent_keepalive: Option<u16>,
}

/// Something the control plane must act on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A STUN or disco datagram arrived on the shared socket.
    Control {
        /// The sender.
        source: SocketAddr,
        /// The datagram.
        datagram: Vec<u8>,
    },
    /// A WireGuard datagram to deliver to `peer` through the relay.
    Relay {
        /// The destination peer.
        peer: NodeId,
        /// The datagram.
        datagram: Vec<u8>,
    },
    /// A new session with `peer` can send: the first one, or a rekey.
    HandshakeCompleted {
        /// The peer.
        peer: NodeId,
    },
    /// No session with `peer` can send any more; it needs a new handshake.
    SessionExpired {
        /// The peer.
        peer: NodeId,
    },
}

/// A snapshot from [`DataplaneHandle::stats`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stats {
    /// Every peer, in `NodeId` order.
    pub peers: Vec<PeerStats>,
    /// Drops since the dataplane started.
    pub drops: DropCounters,
}

/// One peer's session and traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PeerStats {
    /// The network the peer belongs to.
    pub network: NetworkId,
    /// The peer.
    pub node: NodeId,
    /// Where its datagrams go, if known.
    pub endpoint: Option<Endpoint>,
    /// When its latest session was established.
    pub last_handshake: Option<Instant>,
    /// Bytes of authenticated WireGuard datagrams received from it.
    pub rx_bytes: u64,
    /// Bytes of WireGuard datagrams produced for it.
    pub tx_bytes: u64,
}

/// Packets and datagrams dropped, by reason. Every counter saturates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DropCounters {
    /// Outbound packets with no route, or routed to a peer not in the peer list.
    pub no_route: u64,
    /// Datagrams for a peer whose endpoint is unknown.
    pub no_endpoint: u64,
    /// Datagrams and packets WireGuard rejected, and UDP datagrams of no known protocol.
    pub invalid: u64,
    /// Inbound packets from an address outside the sending peer's allowed IPs.
    pub source_not_allowed: u64,
    /// Inbound packets the network's policy refused, or that arrived before it had one.
    pub policy: u64,
    /// Inbound packets that are unparsable or IP fragments.
    pub malformed: u64,
    /// Outbound packets opening a flow while the connection table was full.
    pub table_full: u64,
    /// Packets, datagrams and events that did not fit an output queue.
    pub overflow: u64,
}

/// Why a dataplane operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DataplaneError {
    /// The dataplane task has stopped.
    #[error("the dataplane has stopped")]
    Stopped,
    /// A peer appears twice in one peer list.
    #[error("peer {0} is listed twice")]
    DuplicatePeer(NodeId),
    /// A peer's `NodeId` is not a valid Ed25519 key.
    #[error("peer {0} has an invalid key")]
    InvalidPeerKey(NodeId),
    /// The routes were rejected; nothing changed.
    #[error("routes rejected")]
    Routes(#[from] SetNetworkError),
    /// The exit peer was rejected.
    #[error("exit peer rejected")]
    Exit(#[from] ExitError),
    /// WireGuard refused a peer.
    #[error("WireGuard peer rejected")]
    WireGuard(#[from] WgError),
    /// A packet batch could not be allocated.
    #[error("cannot allocate packet batch")]
    Batch(#[from] BatchError),
    /// Reading the TUN device failed.
    #[error("TUN device failed")]
    Tun(#[from] TunError),
    /// Reading the UDP socket failed.
    #[error("UDP link failed")]
    Udp(#[from] UdpError),
}

type Reply = oneshot::Sender<Result<(), DataplaneError>>;

#[derive(Debug)]
pub(crate) enum Command {
    SetPeers(Vec<PeerSpec>, Reply),
    SetRoutes {
        network: NetworkId,
        mesh: MeshRanges,
        routes: Vec<(IpPrefix, NodeId)>,
        reply: Reply,
    },
    SetExit(Option<(NetworkId, NodeId)>, Reply),
    SetFilter(NetworkId, CompiledFilter),
    SendRaw(SocketAddr, Vec<u8>),
    AcceptRelayed(NodeId, Vec<u8>),
    RemoveNetwork(NetworkId, Reply),
    Stats(oneshot::Sender<Stats>),
}

/// The packet loop. Drive it with [`Dataplane::run`].
#[derive(Debug)]
pub struct Dataplane {
    engine: Engine<Tun>,
}

impl Dataplane {
    /// Builds the dataplane, the handle that commands it, and its event stream.
    pub fn new(
        config: DataplaneConfig,
    ) -> Result<(Self, DataplaneHandle, mpsc::Receiver<Event>), DataplaneError> {
        let (engine, handle, events) = Engine::new(&config.signing_key, config.tun, config.link)?;
        Ok((Self { engine }, handle, events))
    }

    /// Runs until every [`DataplaneHandle`] is dropped or the TUN or UDP read fails.
    pub async fn run(self) -> Result<(), DataplaneError> {
        self.engine.run().await
    }
}

pub(crate) fn channels() -> (
    DataplaneHandle,
    mpsc::Receiver<Command>,
    mpsc::Sender<Event>,
    mpsc::Receiver<Event>,
) {
    let (commands, command_receiver) = mpsc::channel(COMMAND_QUEUE);
    let (events, event_receiver) = mpsc::channel(EVENT_QUEUE);
    (
        DataplaneHandle { commands },
        command_receiver,
        events,
        event_receiver,
    )
}

/// Commands a running [`Dataplane`]. Cheap to clone.
#[derive(Debug, Clone)]
pub struct DataplaneHandle {
    commands: mpsc::Sender<Command>,
}

impl DataplaneHandle {
    /// Replaces the peer list. Peers whose key and keepalive are unchanged keep their sessions.
    pub async fn set_peers(&self, peers: Vec<PeerSpec>) -> Result<(), DataplaneError> {
        self.request(|reply| Command::SetPeers(peers, reply)).await
    }

    /// Replaces `network`'s mesh ranges and routes; a rejected update changes nothing.
    pub async fn set_routes(
        &self,
        network: NetworkId,
        mesh: MeshRanges,
        routes: Vec<(IpPrefix, NodeId)>,
    ) -> Result<(), DataplaneError> {
        self.request(|reply| Command::SetRoutes {
            network,
            mesh,
            routes,
            reply,
        })
        .await
    }

    /// Sends traffic with no specific route to `peer` in `network`, or with `None` stops.
    pub async fn set_exit(&self, exit: Option<(NetworkId, NodeId)>) -> Result<(), DataplaneError> {
        self.request(|reply| Command::SetExit(exit, reply)).await
    }

    /// Sets the inbound policy for `network`. Until set, the network admits no inbound flows.
    pub async fn set_filter(
        &self,
        network: NetworkId,
        filter: CompiledFilter,
    ) -> Result<(), DataplaneError> {
        self.send(Command::SetFilter(network, filter)).await
    }

    /// Sends a raw datagram, such as STUN or disco, from the shared socket.
    pub async fn send_raw(
        &self,
        destination: SocketAddr,
        datagram: Vec<u8>,
    ) -> Result<(), DataplaneError> {
        self.send(Command::SendRaw(destination, datagram)).await
    }

    /// Processes a WireGuard datagram that `peer` sent through the relay.
    pub async fn accept_relayed(
        &self,
        peer: NodeId,
        datagram: Vec<u8>,
    ) -> Result<(), DataplaneError> {
        self.send(Command::AcceptRelayed(peer, datagram)).await
    }

    /// Forgets `network`: its peers and their sessions, its routes, exit and filter.
    pub async fn remove_network(&self, network: NetworkId) -> Result<(), DataplaneError> {
        self.request(|reply| Command::RemoveNetwork(network, reply))
            .await
    }

    /// Per-peer session and traffic figures, and drop counters.
    pub async fn stats(&self) -> Result<Stats, DataplaneError> {
        let (reply, response) = oneshot::channel();
        self.send(Command::Stats(reply)).await?;
        response.await.map_err(|_| DataplaneError::Stopped)
    }

    async fn send(&self, command: Command) -> Result<(), DataplaneError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| DataplaneError::Stopped)
    }

    async fn request(&self, command: impl FnOnce(Reply) -> Command) -> Result<(), DataplaneError> {
        let (reply, response) = oneshot::channel();
        self.send(command(reply)).await?;
        response.await.map_err(|_| DataplaneError::Stopped)?
    }
}
