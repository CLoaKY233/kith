//! The dataplane's public face: configuration, commands and events.

use std::net::{IpAddr, SocketAddr};
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
use crate::tun_device::AnyTun;

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

/// A standard WireGuard interface for peers outside the mesh, sharing the TUN and UDP socket.
#[derive(Clone, PartialEq, Eq)]
pub struct ExternalInterface {
    /// Identifies the interface across updates.
    pub name: String,
    /// The interface's X25519 private key.
    pub private_key: [u8; 32],
    /// The interface's own addresses; packets from them prefer its peers on equal routes.
    pub addresses: Vec<IpPrefix>,
    /// The interface's peers.
    pub peers: Vec<ExternalPeer>,
}

impl core::fmt::Debug for ExternalInterface {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ExternalInterface")
            .field("name", &self.name)
            .field("addresses", &self.addresses)
            .field("peers", &self.peers)
            .finish_non_exhaustive()
    }
}

/// One standard WireGuard peer of an [`ExternalInterface`].
#[derive(Clone, PartialEq, Eq)]
pub struct ExternalPeer {
    /// The peer's X25519 public key.
    pub public_key: [u8; 32],
    /// A preshared key mixed into every handshake.
    pub preshared_key: Option<[u8; 32]>,
    /// Where to send; `None` waits for the peer to send first.
    pub endpoint: Option<SocketAddr>,
    /// Prefixes routed to the peer and accepted from it.
    pub allowed_ips: Vec<IpPrefix>,
    /// Persistent keepalive interval in seconds.
    pub persistent_keepalive: Option<u16>,
    /// Set when the peer is a member of a mesh network, such as a phone: its traffic is
    /// routed to the mesh directly, and what reaches it passes this policy, since it has
    /// no filter of its own. `None` for an imported exit, whose traffic goes to the TUN.
    pub member: Option<ExternalMember>,
}

/// The network a plain WireGuard member belongs to and the policy guarding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalMember {
    /// The member's network.
    pub network: NetworkId,
    /// What may reach the member.
    pub policy: CompiledFilter,
    /// The network's DNS resolver on this device. The member has no resolver of its
    /// own, so its queries there pass whatever the policy says about this device.
    pub resolver: IpAddr,
}

impl core::fmt::Debug for ExternalPeer {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ExternalPeer")
            .field(
                "public_key",
                &kith_crypto::AgreementPublic::from_bytes(self.public_key),
            )
            .field("endpoint", &self.endpoint)
            .field("allowed_ips", &self.allowed_ips)
            .field("persistent_keepalive", &self.persistent_keepalive)
            .field("member", &self.member)
            .finish_non_exhaustive()
    }
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
    /// Every external peer, by interface name, then public key.
    pub external: Vec<ExternalPeerStats>,
    /// Drops since the dataplane started.
    pub drops: DropCounters,
}

/// One external peer's session and traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExternalPeerStats {
    /// The interface's name.
    pub interface: String,
    /// The peer's X25519 public key.
    pub public_key: [u8; 32],
    /// Where its datagrams go, if known.
    pub endpoint: Option<SocketAddr>,
    /// When its latest session was established.
    pub last_handshake: Option<Instant>,
    /// When anything authenticated, keepalives included, last arrived from it.
    pub last_received: Option<Instant>,
    /// Bytes of authenticated WireGuard datagrams received from it.
    pub rx_bytes: u64,
    /// Bytes of WireGuard datagrams produced for it.
    pub tx_bytes: u64,
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
    /// When anything authenticated, keepalives included, last arrived from it.
    pub last_received: Option<Instant>,
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
    /// Outbound exit traffic from an address that is not ours, such as a connection opened
    /// before the exit was turned on; the exit node could not send it on anyway.
    pub foreign_source: u64,
    /// Internet replies from a former exit node arriving just after it stopped being the
    /// exit; their connections had already moved or ended.
    pub after_exit: u64,
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
    /// A public key appears twice in one external interface.
    #[error("external interface {0} lists a peer twice")]
    DuplicateExternalPeer(String),
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
    #[cfg(feature = "test-support")]
    BlockDirectUdp(bool, Reply),
    SetFilter(NetworkId, CompiledFilter),
    SendRaw(SocketAddr, Vec<u8>),
    AcceptRelayed(NodeId, Vec<u8>),
    RemoveNetwork(NetworkId, Reply),
    SetAddresses(NetworkId, Vec<(IpAddr, u8)>, Reply),
    SetExternal(ExternalInterface, Reply),
    RemoveExternal(String, Reply),
    Stats(oneshot::Sender<Stats>),
}

/// The packet loop. Drive it with [`Dataplane::run`].
#[derive(Debug)]
pub struct Dataplane {
    engine: Engine<AnyTun>,
}

impl Dataplane {
    /// Builds the dataplane, the handle that commands it, and its event stream.
    pub fn new(
        config: DataplaneConfig,
    ) -> Result<(Self, DataplaneHandle, mpsc::Receiver<Event>), DataplaneError> {
        let (engine, handle, events) =
            Engine::new(&config.signing_key, AnyTun::Os(config.tun), config.link)?;
        Ok((Self { engine }, handle, events))
    }

    /// Like [`Dataplane::new`], over an in-memory TUN driven through the returned [`crate::MemoryTun`].
    #[cfg(feature = "test-support")]
    pub fn with_memory_tun(
        signing_key: &SigningKey,
        link: UdpLink,
    ) -> Result<
        (
            Self,
            DataplaneHandle,
            mpsc::Receiver<Event>,
            crate::MemoryTun,
        ),
        DataplaneError,
    > {
        let (device, tun) = crate::memory_tun::memory_tun();
        let (engine, handle, events) = Engine::new(signing_key, AnyTun::Memory(device), link)?;
        Ok((Self { engine }, handle, events, tun))
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
    /// Drops every direct UDP datagram in both directions while `blocked`, as a hostile
    /// network would; relayed traffic still flows.
    #[cfg(feature = "test-support")]
    pub async fn block_direct_udp(&self, blocked: bool) -> Result<(), DataplaneError> {
        self.request(|reply| Command::BlockDirectUdp(blocked, reply))
            .await
    }

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

    /// Makes `addresses` (address, prefix length) the TUN addresses held for `network`, adding and removing the difference.
    pub async fn set_addresses(
        &self,
        network: NetworkId,
        addresses: Vec<(IpAddr, u8)>,
    ) -> Result<(), DataplaneError> {
        self.request(|reply| Command::SetAddresses(network, addresses, reply))
            .await
    }

    /// Adds or replaces the external interface named `interface.name`; unchanged peers keep their sessions.
    pub async fn set_external(&self, interface: ExternalInterface) -> Result<(), DataplaneError> {
        self.request(|reply| Command::SetExternal(interface, reply))
            .await
    }

    /// Removes the external interface `name` and its sessions.
    pub async fn remove_external(&self, name: String) -> Result<(), DataplaneError> {
        self.request(|reply| Command::RemoveExternal(name, reply))
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
