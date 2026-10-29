//! The single-task packet loop.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kith_crypto::{SigningKey, VerifyingKey};
use kith_filter::{Counters, Filter, Verdict};
use kith_policy::CompiledFilter;
use kith_router::{PrefixTable, Router};
use kith_transport_udp::{Kind, MAX_SLOT_LEN, RecvBatch, SendBatch, UdpLink, classify};
use kith_tun::{MAX_PACKET_CAPACITY, PacketBatch};
use kith_types::{IpPrefix, NetworkId, NodeId};
use kith_wg::{
    CookieChecker, Decapsulated, Device, Encapsulated, Now, PeerConfig, PeerEvent, PeerId,
    RandomSource, StaticIdentity, Tai64N, Transmit, WgError, required_output_len,
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::api::{
    Command, DataplaneError, DataplaneHandle, DropCounters, Endpoint, Event, ExternalInterface,
    ExternalMember, ExternalPeerStats, PeerSpec, PeerStats, Stats, channels,
};
use crate::packet;
use crate::reassembly::{Complete, Reassembly, is_fragment};
use crate::timers::Timers;
use crate::tun_device::TunDevice;
use kith_packet::{
    IPV6_MIN_MTU, Oversize, clamp_mss, fragment_v4, needs_mss_clamp, oversize, too_big_reply,
};
use kith_pmtu::SizeSearch;

/// Handshakes per second answered before cookies are demanded.
const HANDSHAKES_PER_SECOND: u32 = 1000;

/// The largest ICMP "packet too big" reply: one IPv6 minimum MTU.
const TOO_BIG_REPLY_MAX: usize = 1280;

const _: () = assert!(IPV6_MIN_MTU == 1280);

/// What WireGuard adds to each packet over IPv6, the larger of the two families: IPv6
/// and UDP headers and WireGuard's own 32 bytes. TCP through the tunnel is kept to the
/// tunnel MTU minus this, so with the default of 1280, the IPv6 minimum any working path
/// carries, nothing ever needs a bigger path than that.
const WIREGUARD_OVERHEAD: u16 = 80;

/// A peer whose timer is due but has nothing to send must not spin the loop.
const MIN_TIMER_DELAY: Duration = Duration::from_millis(1);

pub(crate) struct Engine<T> {
    io: Io<T>,
    state: State,
    commands: mpsc::Receiver<Command>,
    clock: Clock,
    addresses: BTreeMap<NetworkId, Vec<(IpAddr, u8)>>,
    /// Set only by tests, to simulate a network that drops direct UDP.
    udp_blocked: bool,
    /// Set only by tests, to simulate a path that drops big datagrams.
    udp_limit: Option<usize>,
}

impl<T> core::fmt::Debug for Engine<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("peers", &self.state.peers.len())
            .finish_non_exhaustive()
    }
}

struct Io<T> {
    tun: T,
    link: UdpLink,
    tun_in: PacketBatch,
    udp_in: RecvBatch,
}

struct State {
    device: Device,
    router: Router,
    filters: BTreeMap<NetworkId, Filter>,
    peers: BTreeMap<NodeId, PeerEntry>,
    by_index: BTreeMap<PeerId, NodeId>,
    externals: BTreeMap<u64, External>,
    external_slots: BTreeMap<String, u64>,
    next_slot: u64,
    timers: Timers<Key>,
    touched: Vec<Key>,
    local: BTreeSet<IpAddr>,
    /// This device's addresses in each network.
    local_in: BTreeMap<NetworkId, BTreeSet<IpAddr>>,
    /// The exit in use until `.2`, whose late internet replies are expected for a while.
    former_exit: Option<(NetworkId, NodeId, Duration)>,
    tun_out: PacketBatch,
    out: Output,
    scratch: Vec<u8>,
    /// A packet a mesh member sent, or one whose MSS is lowered, copied out while routed.
    forward: Vec<u8>,
    /// Where fragments of a packet too big for its path are built.
    fragment: Vec<u8>,
    /// Fragments arriving from peers and members, put back together for the firewall.
    reassembly: Reassembly,
    /// The tunnel MTU, which members' paths are probed for.
    tunnel_mtu: u16,
    /// The IP packet size TCP segments are kept to: what fits a path no larger than the
    /// tunnel MTU itself, once WireGuard's own bytes are added.
    clamp_to: u16,
}

struct PeerEntry {
    id: PeerId,
    network: NetworkId,
    allowed_ips: Vec<IpPrefix>,
    endpoint: Option<Endpoint>,
    keepalive: Option<u16>,
    /// The largest packet the current path carries, when smaller than the tunnel MTU.
    path_mtu: Option<u16>,
    touched: bool,
}

struct External {
    name: String,
    device: Device,
    addresses: Vec<IpPrefix>,
    routes: PrefixTable<PeerId>,
    peers: BTreeMap<PeerId, ExternalEntry>,
}

struct ExternalEntry {
    public_key: [u8; 32],
    preshared_key: Option<[u8; 32]>,
    allowed_ips: Vec<IpPrefix>,
    endpoint: Option<SocketAddr>,
    keepalive: Option<u16>,
    /// For a mesh member, what guards it.
    member: Option<Member>,
    touched: bool,
}

/// A plain WireGuard mesh member behind this device.
struct Member {
    network: NetworkId,
    resolver: IpAddr,
    filter: Filter,
    /// How big a packet the path to it carries. It runs no kith, so it is asked with
    /// padded ICMP echoes through the tunnel, which every phone answers.
    size: SizeSearch,
    /// The echo out now: its sequence number, size and when it left.
    probe: Option<(u16, u16, Duration)>,
    next_sequence: u16,
}

impl Member {
    fn new(spec: ExternalMember) -> Self {
        Self {
            network: spec.network,
            resolver: spec.resolver,
            filter: Filter::new(spec.policy),
            size: SizeSearch::default(),
            probe: None,
            next_sequence: 0,
        }
    }
}

/// Where a packet goes on: a mesh peer, or a peer of an external interface.
#[derive(Clone, Copy)]
enum Hop {
    Mesh(NodeId),
    External(u64, PeerId),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Mesh(NodeId),
    External(u64, PeerId),
}

struct Output {
    udp: SendBatch,
    events: mpsc::Sender<Event>,
    drops: DropCounters,
}

#[derive(Clone, Copy)]
enum Via {
    Udp(SocketAddr),
    Relay(NodeId),
}

enum Wake {
    Command(Command),
    Tun,
    Udp,
    Timer,
}

impl<T: TunDevice> Engine<T> {
    pub(crate) fn new(
        signing_key: &SigningKey,
        tun: T,
        link: UdpLink,
    ) -> Result<(Self, DataplaneHandle, mpsc::Receiver<Event>), DataplaneError> {
        let (handle, commands, events, event_receiver) = channels();
        let clamp_to = tun.mtu().saturating_sub(WIREGUARD_OVERHEAD);
        let secret = signing_key.to_agreement_secret().to_bytes();
        let device = new_device(*secret);
        let engine = Self {
            state: State {
                device,
                router: Router::new(),
                filters: BTreeMap::new(),
                peers: BTreeMap::new(),
                by_index: BTreeMap::new(),
                externals: BTreeMap::new(),
                external_slots: BTreeMap::new(),
                next_slot: 0,
                timers: Timers::new(),
                touched: Vec::with_capacity(kith_tun::BATCH_SIZE),
                local: BTreeSet::new(),
                local_in: BTreeMap::new(),
                former_exit: None,
                tun_out: tun.new_batch()?,
                out: Output {
                    udp: SendBatch::with_capacity(kith_tun::BATCH_SIZE, 256 * 1024),
                    events,
                    drops: DropCounters::default(),
                },
                scratch: vec![0; required_output_len(MAX_PACKET_CAPACITY).max(MAX_SLOT_LEN)],
                forward: Vec::with_capacity(MAX_PACKET_CAPACITY),
                fragment: Vec::new(),
                reassembly: Reassembly::default(),
                tunnel_mtu: tun.mtu(),
                clamp_to,
            },
            io: Io {
                tun_in: tun.new_batch()?,
                udp_in: RecvBatch::compact(&link),
                tun,
                link,
            },
            commands,
            clock: Clock::new(),
            addresses: BTreeMap::new(),
            udp_blocked: false,
            udp_limit: None,
        };
        Ok((engine, handle, event_receiver))
    }

    pub(crate) async fn run(mut self) -> Result<(), DataplaneError> {
        let mut deadline = None;
        loop {
            let timer = deadline.and_then(|due| self.clock.instant(due));
            let wake = tokio::select! {
                command = self.commands.recv() => match command {
                    Some(command) => Wake::Command(command),
                    None => return Ok(()),
                },
                received = self.io.tun.recv_batch(&mut self.io.tun_in) => {
                    received?;
                    Wake::Tun
                }
                received = self.io.link.recv_batch(&mut self.io.udp_in) => {
                    received?;
                    Wake::Udp
                }
                () = sleep_until(timer) => Wake::Timer,
            };
            let now = self.clock.now();
            match wake {
                Wake::Command(command) => self.command(now, command),
                Wake::Tun => self.state.outbound(now, &self.io.tun_in),
                Wake::Udp if self.udp_blocked => {}
                Wake::Udp => self.inbound(now).await,
                Wake::Timer => {}
            }
            deadline = self.state.service(now);
            self.flush().await;
        }
    }

    fn command(&mut self, now: Now, command: Command) {
        #[cfg(feature = "test-support")]
        if let Command::BlockDirectUdp(blocked, reply) = command {
            debug!(blocked, "direct UDP blocked for testing");
            self.udp_blocked = blocked;
            reply.send(Ok(())).ok();
            return;
        }
        #[cfg(feature = "test-support")]
        if let Command::LimitUdpSize(limit, reply) = command {
            debug!(?limit, "big UDP dropped for testing");
            self.udp_limit = limit;
            reply.send(Ok(())).ok();
            return;
        }
        if let Command::SetAddresses(network, addresses, reply) = command {
            reply.send(self.set_addresses(network, addresses)).ok();
            return;
        }
        if let Command::RemoveNetwork(network, _) = &command
            && let Err(error) = self.set_addresses(*network, Vec::new())
        {
            warn!(network = %network.short(), %error, "removing TUN addresses failed");
        }
        self.state.command(now, &self.clock, command);
    }

    fn set_addresses(
        &mut self,
        network: NetworkId,
        addresses: Vec<(IpAddr, u8)>,
    ) -> Result<(), DataplaneError> {
        let previous = self.addresses.remove(&network).unwrap_or_default();
        let held_elsewhere = |address: IpAddr, addresses: &BTreeMap<_, Vec<(IpAddr, u8)>>| {
            addresses
                .values()
                .flatten()
                .any(|&(other, _)| other == address)
        };
        for &(address, _) in &previous {
            if !addresses.iter().any(|&(kept, _)| kept == address)
                && !held_elsewhere(address, &self.addresses)
            {
                self.io.tun.remove_address(address)?;
            }
        }
        let mut applied = Vec::with_capacity(addresses.len());
        let mut result = Ok(());
        for (address, prefix_len) in addresses {
            if let Err(error) = self.io.tun.add_address(address, prefix_len) {
                result = Err(error.into());
                break;
            }
            applied.push((address, prefix_len));
        }
        if !applied.is_empty() {
            debug!(network = %network.short(), addresses = applied.len(), "TUN addresses set");
            self.addresses.insert(network, applied);
        }
        self.state.local = self
            .addresses
            .values()
            .flatten()
            .map(|&(address, _)| address)
            .collect();
        self.state.local_in = self
            .addresses
            .iter()
            .map(|(id, held)| (*id, held.iter().map(|&(address, _)| address).collect()))
            .collect();
        result
    }

    async fn inbound(&mut self, now: Now) {
        for datagram in self.io.udp_in.datagrams() {
            if self
                .udp_limit
                .is_some_and(|limit| datagram.payload.len() > limit)
            {
                continue;
            }
            match classify(datagram.payload) {
                Kind::WireGuard => {
                    if self.state.tun_out.is_full() {
                        self.io.tun.send_batch(&mut self.state.tun_out).await.ok();
                    }
                    self.state
                        .receive(now, Via::Udp(datagram.source), datagram.payload);
                }
                Kind::Stun | Kind::Disco => {
                    self.state.out.emit(Event::Control {
                        source: datagram.source,
                        datagram: datagram.payload.to_vec(),
                    });
                }
                Kind::Unknown => count(&mut self.state.out.drops.invalid),
            }
        }
    }

    /// UDP and TUN writes are best effort, like the networks behind them.
    async fn flush(&mut self) {
        if !self.state.out.udp.is_empty() {
            if !self.udp_blocked {
                self.io.link.send_batch(&self.state.out.udp).await.ok();
            }
            self.state.out.udp.clear();
        }
        if !self.state.tun_out.is_empty() {
            self.io.tun.send_batch(&mut self.state.tun_out).await.ok();
        }
    }
}

impl State {
    fn command(&mut self, now: Now, clock: &Clock, command: Command) {
        match command {
            #[cfg(feature = "test-support")]
            Command::LimitUdpSize(_, reply) => {
                reply.send(Ok(())).ok();
            }
            #[cfg(feature = "test-support")]
            Command::BlockDirectUdp(_, reply) => {
                reply.send(Ok(())).ok();
            }
            Command::SetPeers(peers, reply) => {
                reply.send(self.set_peers(peers)).ok();
            }
            Command::SetPathMtu(node, mtu, reply) => {
                if let Some(entry) = self.peers.get_mut(&node) {
                    entry.path_mtu = mtu;
                }
                reply.send(Ok(())).ok();
            }
            Command::SetRoutes {
                network,
                mesh,
                routes,
                reply,
            } => {
                let count = routes.len();
                let result = self.router.set_network(network, mesh, routes);
                if result.is_ok() {
                    debug!(network = %network.short(), routes = count, "routes set");
                }
                reply.send(result.map_err(DataplaneError::from)).ok();
            }
            Command::SetExit(exit, reply) => {
                reply.send(self.set_exit(now.monotonic, exit)).ok();
            }
            Command::SetFilter(network, policy) => self.set_filter(network, policy),
            Command::SendRaw(destination, datagram) => {
                self.out.send_direct(destination, Some(&datagram));
            }
            Command::AcceptRelayed(peer, datagram) => {
                self.receive(now, Via::Relay(peer), &datagram);
            }
            Command::RemoveNetwork(network, reply) => {
                self.remove_network(network);
                reply.send(Ok(())).ok();
            }
            Command::SetAddresses(_, _, reply) => {
                reply.send(Ok(())).ok();
            }
            Command::SetExternal(interface, reply) => {
                reply.send(self.set_external(interface)).ok();
            }
            Command::RemoveExternal(name, reply) => {
                self.remove_external(&name);
                reply.send(Ok(())).ok();
            }
            Command::Stats(reply) => {
                reply.send(self.stats(clock)).ok();
            }
        }
    }

    fn set_peers(&mut self, peers: Vec<PeerSpec>) -> Result<(), DataplaneError> {
        let mut wanted = BTreeMap::new();
        for spec in peers {
            let key = VerifyingKey::from_node_id(spec.node)
                .map_err(|_| DataplaneError::InvalidPeerKey(spec.node))?
                .to_agreement_public()
                .to_bytes();
            let node = spec.node;
            if wanted.insert(node, (spec, key)).is_some() {
                return Err(DataplaneError::DuplicatePeer(node));
            }
        }
        let stale: Vec<NodeId> = self
            .peers
            .iter()
            .filter(|&(node, entry)| {
                wanted
                    .get(node)
                    .is_none_or(|(spec, _)| spec.persistent_keepalive != entry.keepalive)
            })
            .map(|(node, _)| *node)
            .collect();
        for node in stale {
            self.remove_peer(node);
        }
        for (node, (spec, key)) in wanted {
            if let Some(entry) = self.peers.get_mut(&node) {
                entry.network = spec.network;
                entry.allowed_ips = spec.allowed_ips;
                if spec.endpoint.is_some() && spec.endpoint != entry.endpoint {
                    debug!(peer = %node.short(), endpoint = ?spec.endpoint, "peer endpoint set");
                    let now_direct = matches!(spec.endpoint, Some(Endpoint::Direct(_)))
                        && !matches!(entry.endpoint, Some(Endpoint::Direct(_)));
                    if (entry.endpoint.is_none() || now_direct)
                        && let Some(peer) = self.device.peer_mut(entry.id)
                    {
                        peer.request_handshake();
                        touch(&mut self.touched, Key::Mesh(node), &mut entry.touched);
                    }
                    entry.endpoint = spec.endpoint;
                }
                continue;
            }
            let mut config = PeerConfig::new(key);
            if let Some(seconds) = spec.persistent_keepalive {
                config = config.with_persistent_keepalive(seconds);
            }
            let externals = &self.externals;
            let id = add_unique(&mut self.device, &config, |id| {
                externals
                    .values()
                    .any(|external| external.peers.contains_key(&id))
            })?;
            debug!(peer = %node.short(), network = %spec.network.short(), "WireGuard peer added");
            if spec.endpoint.is_some()
                && let Some(peer) = self.device.peer_mut(id)
            {
                peer.request_handshake();
            }
            self.by_index.insert(id, node);
            self.peers.insert(
                node,
                PeerEntry {
                    id,
                    network: spec.network,
                    allowed_ips: spec.allowed_ips,
                    endpoint: spec.endpoint,
                    keepalive: spec.persistent_keepalive,
                    path_mtu: None,
                    touched: true,
                },
            );
            self.touched.push(Key::Mesh(node));
        }
        debug_assert_eq!(
            self.by_index.len(),
            self.peers.len(),
            "the session index and the peer table disagree"
        );
        Ok(())
    }

    fn remove_peer(&mut self, node: NodeId) {
        let Some(entry) = self.peers.remove(&node) else {
            return;
        };
        self.device.remove_peer(entry.id);
        self.by_index.remove(&entry.id);
        self.timers.cancel(Key::Mesh(node));
        debug!(peer = %node.short(), network = %entry.network.short(), "WireGuard peer removed");
    }

    fn remove_network(&mut self, network: NetworkId) {
        let members: Vec<NodeId> = self
            .peers
            .iter()
            .filter(|(_, entry)| entry.network == network)
            .map(|(node, _)| *node)
            .collect();
        for node in &members {
            self.remove_peer(*node);
        }
        self.router.remove_network(network);
        if let Some(filter) = self.filters.remove(&network) {
            absorb(&mut self.out.drops, filter.counters());
        }
        debug!(network = %network.short(), peers = members.len(), "network removed");
    }

    fn set_exit(
        &mut self,
        now: Duration,
        exit: Option<(NetworkId, NodeId)>,
    ) -> Result<(), DataplaneError> {
        if let Some((network, peer)) = self
            .router
            .exit_peer()
            .filter(|current| Some(*current) != exit)
        {
            self.former_exit = Some((network, peer, now));
        }
        if let Some((network, peer)) = exit {
            self.router.set_exit_peer(network, Some(peer))?;
            self.router.set_active_exit(Some(network))?;
            debug!(network = %network.short(), peer = %peer.short(), "exit node set");
        } else {
            self.router.set_active_exit(None)?;
            debug!("exit node cleared");
        }
        Ok(())
    }

    fn set_filter(&mut self, network: NetworkId, policy: CompiledFilter) {
        match self.filters.get_mut(&network) {
            Some(filter) => filter.set_policy(policy),
            None => {
                self.filters.insert(network, Filter::new(policy));
            }
        }
        debug!(network = %network.short(), "filter set");
    }

    fn stats(&mut self, clock: &Clock) -> Stats {
        let mut peers = Vec::with_capacity(self.peers.len());
        for (node, entry) in &self.peers {
            let Some(peer) = self.device.peer_mut(entry.id) else {
                continue;
            };
            peers.push(PeerStats {
                network: entry.network,
                node: *node,
                endpoint: entry.endpoint,
                last_handshake: peer.last_handshake().and_then(|at| clock.at(at)),
                last_received: peer.last_received().and_then(|at| clock.at(at)),
                rx_bytes: peer.rx_bytes(),
                tx_bytes: peer.tx_bytes(),
            });
        }
        let mut external = Vec::new();
        for interface in self.externals.values_mut() {
            for (id, entry) in &interface.peers {
                let Some(peer) = interface.device.peer_mut(*id) else {
                    continue;
                };
                external.push(ExternalPeerStats {
                    interface: interface.name.clone(),
                    public_key: entry.public_key,
                    endpoint: entry.endpoint,
                    last_handshake: peer.last_handshake().and_then(|at| clock.at(at)),
                    last_received: peer.last_received().and_then(|at| clock.at(at)),
                    rx_bytes: peer.rx_bytes(),
                    tx_bytes: peer.tx_bytes(),
                    path_mtu: entry.member.as_ref().and_then(|member| member.size.limit()),
                });
            }
        }
        external.sort_by(|a, b| (&a.interface, a.public_key).cmp(&(&b.interface, b.public_key)));
        let mut drops = self.out.drops;
        for filter in self.filters.values() {
            absorb(&mut drops, filter.counters());
        }
        for external in self.externals.values() {
            for member in external
                .peers
                .values()
                .filter_map(|entry| entry.member.as_ref())
            {
                absorb(&mut drops, member.filter.counters());
            }
        }
        Stats {
            peers,
            external,
            drops,
        }
    }

    fn outbound(&mut self, now: Now, batch: &PacketBatch) {
        for packet in batch.iter() {
            if needs_mss_clamp(packet, self.clamp_to) {
                let mut copy = core::mem::take(&mut self.forward);
                copy.clear();
                copy.extend_from_slice(packet);
                clamp_mss(&mut copy, self.clamp_to);
                self.outbound_packet(now, &copy);
                self.forward = copy;
            } else {
                self.outbound_packet(now, packet);
            }
        }
    }

    /// Sends a packet from the TUN to whoever its destination routes to.
    fn outbound_packet(&mut self, now: Now, packet: &[u8]) {
        let Some(destination) = packet::destination(packet) else {
            count(&mut self.out.drops.no_route);
            return;
        };
        if self.local.contains(&destination) {
            if self.tun_out.push(packet).is_err() {
                count(&mut self.out.drops.no_route);
            }
            return;
        }
        let route = self.router.route(destination);
        let mesh_rank = route.map(|route| (route.prefix.prefix_len(), false));
        if let Some((slot, id)) =
            self.external_route(packet::source(packet), destination, mesh_rank)
        {
            if self.guard_member(now, packet, slot, id) {
                self.outbound_external(now, packet, slot, id);
            }
            return;
        }
        let Some(route) = route else {
            count(&mut self.out.drops.no_route);
            return;
        };
        if route.prefix.prefix_len() == 0
            && packet::source(packet).is_some_and(|source| !self.local.contains(&source))
        {
            count(&mut self.out.drops.foreign_source);
            return;
        }
        if let Some(filter) = self.filters.get_mut(&route.network)
            && filter.outbound(packet, now.monotonic) != Verdict::Accept
        {
            return;
        }
        self.seal_to_peer(now, packet, route.peer);
    }

    /// Encrypts `packet` for mesh peer `node` and sends it, or, when it is bigger than the
    /// path to `node` carries, splits it or tells its sender, as a router would.
    fn seal_to_peer(&mut self, now: Now, packet: &[u8], node: NodeId) {
        let Some(entry) = self.peers.get_mut(&node) else {
            count(&mut self.out.drops.no_route);
            return;
        };
        if let Some(mtu) = entry
            .path_mtu
            .filter(|mtu| packet.len() > usize::from(*mtu))
        {
            self.oversized(now, packet, mtu, Hop::Mesh(node));
            return;
        }
        let Some(peer) = self.device.peer_mut(entry.id) else {
            count(&mut self.out.drops.no_route);
            return;
        };
        let sealed = match self.scratch.get_mut(..required_output_len(packet.len())) {
            Some(out) => peer.encapsulate(now, packet, out),
            None => Err(WgError::BufferTooSmall),
        };
        match sealed {
            Ok(Encapsulated::Transport(len) | Encapsulated::HandshakeInitiation(len)) => {
                self.out.send(node, entry.endpoint, self.scratch.get(..len));
            }
            Ok(Encapsulated::Queued) => {}
            Err(_) => count(&mut self.out.drops.invalid),
        }
        touch(&mut self.touched, Key::Mesh(node), &mut entry.touched);
    }

    /// Handles a packet bigger than the `mtu` the path to `to` carries: an IPv4 packet that
    /// allows it goes on in fragments; for anything else the sender, here or behind us, is
    /// told the size with ICMP and the packet is dropped.
    fn oversized(&mut self, now: Now, packet: &[u8], mtu: u16, to: Hop) {
        match oversize(packet) {
            Oversize::Fragment => {
                let mut scratch = core::mem::take(&mut self.fragment);
                scratch.resize(usize::from(mtu), 0);
                let mut pieces = Vec::new();
                fragment_v4(packet, mtu, &mut scratch, |piece| {
                    pieces.push(piece.to_vec());
                });
                self.fragment = scratch;
                for piece in pieces {
                    match to {
                        Hop::Mesh(node) => self.seal_to_peer(now, &piece, node),
                        Hop::External(slot, id) => self.outbound_external(now, &piece, slot, id),
                    }
                }
            }
            Oversize::Reply => {
                count(&mut self.out.drops.too_big);
                let mut reply = [0_u8; TOO_BIG_REPLY_MAX];
                if let Some(len) = too_big_reply(packet, mtu, &mut reply)
                    && let Some(reply) = reply.get(..len)
                {
                    self.outbound_packet(now, reply);
                }
            }
            Oversize::Drop => count(&mut self.out.drops.too_big),
        }
    }

    /// Whether `packet` may reach external peer `id`: always for an imported exit, and
    /// for a mesh member only if its network's policy lets it in.
    fn guard_member(&mut self, now: Now, packet: &[u8], slot: u64, id: PeerId) -> bool {
        let member = self
            .externals
            .get_mut(&slot)
            .and_then(|external| external.peers.get_mut(&id))
            .and_then(|entry| entry.member.as_mut());
        member.is_none_or(|member| member.filter.inbound(packet, now.monotonic) == Verdict::Accept)
    }

    /// Routes a packet a mesh member sent: to its network's resolver, to this device's
    /// address in its own network (never in another one), to another member behind this
    /// one, across the mesh, or to the OS for the internet. Its replies may come back
    /// because its own filter tracks the flow.
    fn route_from_member(
        &mut self,
        now: Now,
        (whole, parts): (&[u8], Option<&[Vec<u8>]>),
        network: NetworkId,
        resolver: IpAddr,
    ) {
        let Some(destination) = packet::destination(whole) else {
            count(&mut self.out.drops.malformed);
            return;
        };
        if destination == resolver {
            self.push_each(whole, parts);
            return;
        }
        let ours = self
            .local_in
            .get(&network)
            .is_some_and(|held| held.contains(&destination));
        if !ours && self.local.contains(&destination) {
            count(&mut self.out.drops.policy);
            return;
        }
        if ours {
            let admitted = if let Some(filter) = self.filters.get_mut(&network) {
                filter.inbound(whole, now.monotonic) == Verdict::Accept
            } else {
                count(&mut self.out.drops.policy);
                false
            };
            if admitted {
                self.push_each(whole, parts);
            }
            return;
        }
        let route = self.router.route(destination);
        let mesh_rank = route.map(|route| (route.prefix.prefix_len(), false));
        if let Some((slot, id)) = self.external_route(packet::source(whole), destination, mesh_rank)
        {
            if self.guard_member(now, whole, slot, id) {
                for part in each(whole, parts) {
                    self.outbound_external(now, part, slot, id);
                }
            }
            return;
        }
        match route {
            Some(route) => {
                for part in each(whole, parts) {
                    self.seal_to_peer(now, part, route.peer);
                }
            }
            None => self.push_each(whole, parts),
        }
    }

    /// Writes a packet, or the fragments it arrived in, to the TUN.
    fn push_each(&mut self, whole: &[u8], parts: Option<&[Vec<u8>]>) {
        for part in each(whole, parts) {
            if self.tun_out.push(part).is_err() {
                count(&mut self.out.drops.overflow);
            }
        }
    }

    /// Delivers a datagram that arrived from mesh peer `node` in fragments: judged whole,
    /// sent on as the fragments.
    fn deliver_reassembled(&mut self, now: Now, node: NodeId, role: ExitRole, done: &Complete) {
        let Some(entry) = self.peers.get(&node) else {
            return;
        };
        let member = packet::destination(&done.whole)
            .and_then(|destination| member_route(&self.externals, destination));
        match member {
            Some((slot, id)) => {
                if owns_source(entry, role, &mut self.out.drops, &done.whole)
                    && self.guard_member(now, &done.whole, slot, id)
                {
                    for part in &done.parts {
                        self.outbound_external(now, part, slot, id);
                    }
                }
            }
            None => {
                if admits(
                    entry,
                    role,
                    &mut self.filters,
                    &mut self.out.drops,
                    &done.whole,
                    now.monotonic,
                ) {
                    self.push_each(&done.whole, Some(&done.parts));
                }
            }
        }
    }

    fn external_route(
        &self,
        source: Option<IpAddr>,
        destination: IpAddr,
        mesh_rank: Option<(u8, bool)>,
    ) -> Option<(u64, PeerId)> {
        let mut best: Option<((u8, bool), u64, PeerId)> = None;
        for (slot, external) in &self.externals {
            let Some((prefix, id)) = external.routes.lookup(destination) else {
                continue;
            };
            let owns_source = source.is_some_and(|source| {
                external
                    .addresses
                    .iter()
                    .any(|address| address.contains(source))
            });
            let rank = (prefix.prefix_len(), owns_source);
            if best.is_none_or(|(current, _, _)| rank > current) {
                best = Some((rank, *slot, *id));
            }
        }
        best.filter(|(rank, _, _)| mesh_rank.is_none_or(|mesh| *rank > mesh))
            .map(|(_, slot, id)| (slot, id))
    }

    fn outbound_external(&mut self, now: Now, packet: &[u8], slot: u64, id: PeerId) {
        let Some(external) = self.externals.get_mut(&slot) else {
            return;
        };
        let limit = external
            .peers
            .get(&id)
            .and_then(|entry| entry.member.as_ref())
            .and_then(|member| member.size.limit())
            .filter(|mtu| packet.len() > usize::from(*mtu));
        if let Some(mtu) = limit {
            self.oversized(now, packet, mtu, Hop::External(slot, id));
            return;
        }
        let Some(external) = self.externals.get_mut(&slot) else {
            return;
        };
        let (Some(entry), Some(peer)) = (external.peers.get_mut(&id), external.device.peer_mut(id))
        else {
            count(&mut self.out.drops.no_route);
            return;
        };
        let sealed = match self.scratch.get_mut(..required_output_len(packet.len())) {
            Some(out) => peer.encapsulate(now, packet, out),
            None => Err(WgError::BufferTooSmall),
        };
        match sealed {
            Ok(Encapsulated::Transport(len) | Encapsulated::HandshakeInitiation(len)) => {
                self.out
                    .send_external(entry.endpoint, self.scratch.get(..len));
            }
            Ok(Encapsulated::Queued) => {}
            Err(_) => count(&mut self.out.drops.invalid),
        }
        touch(
            &mut self.touched,
            Key::External(slot, id),
            &mut entry.touched,
        );
    }

    fn receive_external(&mut self, now: Now, source: SocketAddr, datagram: &[u8]) {
        let mut address = [0; 18];
        let encoded = encode_address(source, &mut address);
        let mut member_packet = None;
        let mut handled = false;
        for (slot, external) in &mut self.externals {
            let output =
                match external
                    .device
                    .decapsulate(now, encoded, datagram, &mut self.scratch)
                {
                    Ok(output) => output,
                    Err(WgError::UnknownIndex | WgError::InvalidMac) => continue,
                    Err(_) => break,
                };
            let Some(id) = output.peer else {
                if let Decapsulated::Reply(len) = output.result {
                    self.out.send_direct(source, self.scratch.get(..len));
                }
                return;
            };
            let Some(entry) = external.peers.get_mut(&id) else {
                return;
            };
            if entry.endpoint != Some(source) {
                debug!(interface = %external.name, %source, "external peer endpoint learned");
                entry.endpoint = Some(source);
            }
            match output.result {
                Decapsulated::Packet(len) => {
                    if let Some(packet) = self.scratch.get_mut(..len) {
                        clamp_mss(packet, self.clamp_to);
                    }
                    if let Some(packet) = self.scratch.get(..len) {
                        if !packet::source(packet).is_some_and(|address| {
                            entry
                                .allowed_ips
                                .iter()
                                .any(|prefix| prefix.contains(address))
                        }) {
                            count(&mut self.out.drops.source_not_allowed);
                        } else if let Some(member) = entry.member.as_mut()
                            && let Some((sequence, size, _)) = member.probe
                            && echo_reply_sequence(packet) == Some(sequence)
                        {
                            member.size.answered(size);
                            member.probe = None;
                        } else if let Some(member) = entry.member.as_mut() {
                            let fragment = is_fragment(packet);
                            let assembled = if fragment {
                                self.reassembly.add(packet, now.monotonic)
                            } else {
                                None
                            };
                            if !fragment || assembled.is_some() {
                                let judged =
                                    assembled.as_ref().map_or(packet, |done| &done.whole[..]);
                                member.filter.outbound(judged, now.monotonic);
                                member_packet =
                                    Some((member.network, member.resolver, len, assembled));
                            }
                        } else if self.tun_out.push(packet).is_err() {
                            count(&mut self.out.drops.overflow);
                        }
                    }
                }
                Decapsulated::Reply(len) => self.out.send_direct(source, self.scratch.get(..len)),
                Decapsulated::Nothing => {}
            }
            touch(
                &mut self.touched,
                Key::External(*slot, id),
                &mut entry.touched,
            );
            handled = true;
            break;
        }
        if !handled {
            count(&mut self.out.drops.invalid);
            return;
        }
        let Some((network, resolver, len, assembled)) = member_packet else {
            return;
        };
        if let Some(done) = assembled {
            self.route_from_member(now, (&done.whole, Some(&done.parts)), network, resolver);
            return;
        }
        let mut packet = core::mem::take(&mut self.forward);
        packet.clear();
        packet.extend_from_slice(self.scratch.get(..len).unwrap_or_default());
        self.route_from_member(now, (&packet, None), network, resolver);
        self.forward = packet;
    }

    fn set_external(&mut self, spec: ExternalInterface) -> Result<(), DataplaneError> {
        let mut wanted = BTreeMap::new();
        for peer in spec.peers {
            if wanted.insert(peer.public_key, peer).is_some() {
                return Err(DataplaneError::DuplicateExternalPeer(spec.name));
            }
        }
        let (slot, mut external) = self.take_external(&spec.name, spec.private_key);
        let stale: Vec<PeerId> = external
            .peers
            .iter()
            .filter(|(_, entry)| {
                wanted.get(&entry.public_key).is_none_or(|peer| {
                    peer.preshared_key != entry.preshared_key
                        || peer.persistent_keepalive != entry.keepalive
                })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            external.peers.remove(&id);
            external.device.remove_peer(id);
            self.timers.cancel(Key::External(slot, id));
        }
        let mut result = Ok(());
        for (public_key, peer) in wanted {
            if let Some(id) = external.device.peer_by_public_key(&public_key)
                && let Some(entry) = external.peers.get_mut(&id)
            {
                entry.allowed_ips = peer.allowed_ips;
                if peer.endpoint.is_some() {
                    entry.endpoint = peer.endpoint;
                }
                entry.member = match (entry.member.take(), peer.member) {
                    (Some(mut kept), Some(spec)) if kept.network == spec.network => {
                        kept.filter.set_policy(spec.policy);
                        kept.resolver = spec.resolver;
                        Some(kept)
                    }
                    (_, spec) => spec.map(Member::new),
                };
                continue;
            }
            let mut config = PeerConfig::new(public_key);
            if let Some(key) = peer.preshared_key {
                config = config.with_preshared_key(key);
            }
            if let Some(seconds) = peer.persistent_keepalive {
                config = config.with_persistent_keepalive(seconds);
            }
            let (by_index, externals) = (&self.by_index, &self.externals);
            let id = match add_unique(&mut external.device, &config, |id| {
                by_index.contains_key(&id)
                    || externals
                        .values()
                        .any(|other| other.peers.contains_key(&id))
            }) {
                Ok(id) => id,
                Err(error) => {
                    result = Err(error.into());
                    break;
                }
            };
            external.peers.insert(
                id,
                ExternalEntry {
                    public_key,
                    preshared_key: peer.preshared_key,
                    allowed_ips: peer.allowed_ips,
                    endpoint: peer.endpoint,
                    keepalive: peer.persistent_keepalive,
                    member: peer.member.map(Member::new),
                    touched: true,
                },
            );
            self.touched.push(Key::External(slot, id));
        }
        external.addresses = spec.addresses;
        external.routes = PrefixTable::new();
        for (id, entry) in &external.peers {
            for prefix in &entry.allowed_ips {
                external.routes.insert(*prefix, *id);
            }
        }
        debug!(interface = %external.name, peers = external.peers.len(), "external interface set");
        self.external_slots.insert(spec.name, slot);
        self.externals.insert(slot, external);
        result
    }

    fn take_external(&mut self, name: &str, private_key: [u8; 32]) -> (u64, External) {
        let public = StaticIdentity::from_secret_bytes(private_key).public_key();
        if let Some(&slot) = self.external_slots.get(name)
            && self
                .externals
                .get(&slot)
                .is_some_and(|external| external.device.public_key() == public)
            && let Some(external) = self.externals.remove(&slot)
        {
            return (slot, external);
        }
        self.remove_external(name);
        let slot = self.next_slot;
        self.next_slot = slot.wrapping_add(1);
        let external = External {
            name: name.to_owned(),
            device: new_device(private_key),
            addresses: Vec::new(),
            routes: PrefixTable::new(),
            peers: BTreeMap::new(),
        };
        (slot, external)
    }

    fn remove_external(&mut self, name: &str) {
        let Some(slot) = self.external_slots.remove(name) else {
            return;
        };
        if let Some(external) = self.externals.remove(&slot) {
            for id in external.peers.keys() {
                self.timers.cancel(Key::External(slot, *id));
            }
            debug!(interface = %name, "external interface removed");
        }
    }

    fn receive(&mut self, now: Now, via: Via, datagram: &[u8]) {
        let mut address = [0; 18];
        let source = match &via {
            Via::Udp(source) => encode_address(*source, &mut address),
            Via::Relay(peer) => peer.as_bytes().as_slice(),
        };
        let output = match self
            .device
            .decapsulate(now, source, datagram, &mut self.scratch)
        {
            Ok(output) => output,
            Err(WgError::UnknownIndex | WgError::InvalidMac) if !self.externals.is_empty() => {
                if let Via::Udp(source) = via {
                    self.receive_external(now, source, datagram);
                } else {
                    count(&mut self.out.drops.invalid);
                }
                return;
            }
            Err(_) => {
                count(&mut self.out.drops.invalid);
                return;
            }
        };
        let Some(id) = output.peer else {
            if let Decapsulated::Reply(len) = output.result {
                self.out.reply(via, self.scratch.get(..len));
            }
            return;
        };
        let Some(&node) = self.by_index.get(&id) else {
            return;
        };
        let Some(entry) = self.peers.get_mut(&node) else {
            return;
        };
        if let Via::Udp(source) = via {
            let endpoint = Some(Endpoint::Direct(source));
            if entry.endpoint != endpoint {
                debug!(peer = %node.short(), %source, "peer endpoint learned");
                entry.endpoint = endpoint;
            }
        }
        let role = exit_role(
            self.router.exit_peer(),
            self.former_exit,
            (entry.network, node),
            now,
        );
        let mut for_member = None;
        let mut reassembled = None;
        match output.result {
            Decapsulated::Packet(len) => {
                if let Some(packet) = self.scratch.get_mut(..len) {
                    clamp_mss(packet, self.clamp_to);
                }
                if let Some(packet) = self.scratch.get(..len)
                    && is_fragment(packet)
                {
                    reassembled = self.reassembly.add(packet, now.monotonic);
                } else if let Some(packet) = self.scratch.get(..len) {
                    let member = packet::destination(packet)
                        .and_then(|destination| member_route(&self.externals, destination));
                    match member {
                        Some(route) => {
                            if owns_source(entry, role, &mut self.out.drops, packet) {
                                for_member = Some((route, len));
                            }
                        }
                        None => {
                            if admits(
                                entry,
                                role,
                                &mut self.filters,
                                &mut self.out.drops,
                                packet,
                                now.monotonic,
                            ) && self.tun_out.push(packet).is_err()
                            {
                                count(&mut self.out.drops.overflow);
                            }
                        }
                    }
                }
            }
            Decapsulated::Reply(len) => self.out.reply(via, self.scratch.get(..len)),
            Decapsulated::Nothing => {}
        }
        touch(&mut self.touched, Key::Mesh(node), &mut entry.touched);
        if let Some(done) = reassembled {
            self.deliver_reassembled(now, node, role, &done);
        }
        if let Some(((slot, id), len)) = for_member {
            let mut packet = core::mem::take(&mut self.forward);
            packet.clear();
            packet.extend_from_slice(self.scratch.get(..len).unwrap_or_default());
            if self.guard_member(now, &packet, slot, id) {
                self.outbound_external(now, &packet, slot, id);
            }
            self.forward = packet;
        }
    }

    fn service(&mut self, now: Now) -> Option<Duration> {
        while let Some(key) = self.timers.pop_due(now.monotonic) {
            let flag = match key {
                Key::Mesh(node) => self.peers.get_mut(&node).map(|entry| &mut entry.touched),
                Key::External(slot, id) => self
                    .externals
                    .get_mut(&slot)
                    .and_then(|external| external.peers.get_mut(&id))
                    .map(|entry| &mut entry.touched),
            };
            if let Some(flag) = flag {
                touch(&mut self.touched, key, flag);
            }
        }
        let mut touched = core::mem::take(&mut self.touched);
        for key in touched.drain(..) {
            match key {
                Key::Mesh(node) => self.service_peer(now, node),
                Key::External(slot, id) => self.service_external(now, slot, id),
            }
        }
        self.touched = touched;
        self.timers.next_deadline()
    }

    fn service_peer(&mut self, now: Now, node: NodeId) {
        let Some(entry) = self.peers.get_mut(&node) else {
            return;
        };
        entry.touched = false;
        let Some(peer) = self.device.peer_mut(entry.id) else {
            return;
        };
        let at = now.monotonic;
        if peer.poll_timeout().is_some_and(|due| due <= at) {
            while let Ok(Transmit::Send(len)) = peer.poll_transmit(now, &mut self.scratch) {
                self.out.send(node, entry.endpoint, self.scratch.get(..len));
            }
        }
        while let Some(event) = peer.poll_event() {
            self.out.session_event(node, entry.network, event);
        }
        if let Some(due) = peer.poll_timeout() {
            self.timers
                .schedule(Key::Mesh(node), due.max(at.saturating_add(MIN_TIMER_DELAY)));
        }
    }

    fn service_external(&mut self, now: Now, slot: u64, id: PeerId) {
        let Some(external) = self.externals.get_mut(&slot) else {
            return;
        };
        let (Some(entry), Some(peer)) = (external.peers.get_mut(&id), external.device.peer_mut(id))
        else {
            return;
        };
        entry.touched = false;
        let at = now.monotonic;
        if peer.poll_timeout().is_some_and(|due| due <= at) {
            while let Ok(Transmit::Send(len)) = peer.poll_transmit(now, &mut self.scratch) {
                self.out
                    .send_external(entry.endpoint, self.scratch.get(..len));
            }
        }
        while let Some(event) = peer.poll_event() {
            debug!(interface = %external.name, ?event, "external WireGuard session event");
        }
        let mut due = peer.poll_timeout();
        let heard = peer
            .last_received()
            .is_some_and(|last| at.saturating_sub(last) < MEMBER_SILENCE);
        if let Some(member) = entry.member.as_mut() {
            if let Some((_, size, sent)) = member.probe
                && at.saturating_sub(sent) >= PROBE_TIMEOUT
            {
                member.size.lost(size);
                member.probe = None;
            }
            if member.size.target() != self.tunnel_mtu {
                member.size = SizeSearch::new(self.tunnel_mtu);
            }
            let source = self
                .local_in
                .get(&member.network)
                .and_then(|held| held.iter().find(|address| address.is_ipv4()).copied());
            let destination = entry.allowed_ips.iter().find_map(|prefix| match prefix {
                IpPrefix::V4(v4) if v4.prefix_len() == 32 => Some(IpAddr::V4(v4.addr())),
                IpPrefix::V4(_) | IpPrefix::V6(_) => None,
            });
            if heard
                && member.probe.is_none()
                && let (Some(source), Some(destination)) = (source, destination)
                && let Some(size) = member.size.next_probe(at)
            {
                let sequence = member.next_sequence;
                member.next_sequence = sequence.wrapping_add(1);
                let echo = echo_request(source, destination, sequence, size);
                if let Some(out) = self.scratch.get_mut(..required_output_len(echo.len()))
                    && let Ok(Encapsulated::Transport(len)) = peer.encapsulate(now, &echo, out)
                {
                    member.probe = Some((sequence, size, at));
                    self.out
                        .send_external(entry.endpoint, self.scratch.get(..len));
                } else {
                    member.size.lost(size);
                }
            }
            let probe_due = member
                .probe
                .map(|(_, _, sent)| sent.saturating_add(PROBE_TIMEOUT))
                .or_else(|| heard.then(|| member.size.next_due(at)).flatten());
            due = [due, probe_due].into_iter().flatten().min();
        }
        if let Some(due) = due {
            self.timers.schedule(
                Key::External(slot, id),
                due.max(at.saturating_add(MIN_TIMER_DELAY)),
            );
        }
    }
}

fn touch(touched: &mut Vec<Key>, key: Key, flag: &mut bool) {
    if !*flag {
        *flag = true;
        touched.push(key);
    }
}

fn new_device(secret: [u8; 32]) -> Device {
    Device::new(
        secret,
        CookieChecker::new(HANDSHAKES_PER_SECOND),
        Box::new(OsRandom),
    )
}

const UNIQUE_ATTEMPTS: usize = 8;

fn add_unique(
    device: &mut Device,
    config: &PeerConfig,
    taken: impl Fn(PeerId) -> bool,
) -> Result<PeerId, WgError> {
    for _ in 0..UNIQUE_ATTEMPTS {
        let id = device.add_peer(config, Box::new(OsRandom))?;
        if !taken(id) {
            return Ok(id);
        }
        device.remove_peer(id);
    }
    Err(WgError::PeerRejected)
}

fn count(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

fn absorb(drops: &mut DropCounters, filter: Counters) {
    drops.policy = drops.policy.saturating_add(filter.dropped_policy);
    drops.malformed = drops.malformed.saturating_add(filter.dropped_malformed);
    drops.table_full = drops.table_full.saturating_add(filter.dropped_table_full);
}

/// How a peer relates to the exit, for which sources its packets may carry.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExitRole {
    /// The exit in use: any source, since replies come from the internet.
    Current,
    /// The exit until moments ago: internet sources are late replies, dropped quietly.
    Former,
    None,
}

/// How long internet replies from a former exit count as expected.
const AFTER_EXIT_GRACE: Duration = Duration::from_secs(120);

/// How `peer` relates to the exit, given the exit in use and the one before it.
fn exit_role(
    current: Option<(NetworkId, NodeId)>,
    former: Option<(NetworkId, NodeId, Duration)>,
    peer: (NetworkId, NodeId),
    now: Now,
) -> ExitRole {
    if current == Some(peer) {
        ExitRole::Current
    } else if former.is_some_and(|(network, node, until)| {
        (network, node) == peer && now.monotonic.saturating_sub(until) < AFTER_EXIT_GRACE
    }) {
        ExitRole::Former
    } else {
        ExitRole::None
    }
}

/// The ICMP identifier of a gateway's size probes to its plain WireGuard members.
const PROBE_ID: u16 = 0x6b70;
/// How long an echo probe may take to come back.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a plain WireGuard member, which sends a keepalive every 25s, may stay silent
/// and still be probed.
const MEMBER_SILENCE: Duration = Duration::from_secs(60);

/// An ICMP echo request of `size` bytes from `source` to `destination`: a size probe.
fn echo_request(source: IpAddr, destination: IpAddr, sequence: u16, size: u16) -> Vec<u8> {
    let mut packet = vec![0_u8; usize::from(size.max(28))];
    let (IpAddr::V4(source), IpAddr::V4(destination)) = (source, destination) else {
        return packet;
    };
    let total = u16::try_from(packet.len()).unwrap_or(u16::MAX);
    let header: [u8; 20] = {
        let mut header = [
            0x45, 0, 0, 0, 0, 0, 0x40, 0, 64, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        if let Some(field) = header.get_mut(2..4) {
            field.copy_from_slice(&total.to_be_bytes());
        }
        if let Some(field) = header.get_mut(12..16) {
            field.copy_from_slice(&source.octets());
        }
        if let Some(field) = header.get_mut(16..20) {
            field.copy_from_slice(&destination.octets());
        }
        header
    };
    if let Some(field) = packet.get_mut(..20) {
        field.copy_from_slice(&header);
    }
    if let Some(icmp) = packet.get_mut(20..28) {
        icmp.copy_from_slice(&[8, 0, 0, 0, 0, 0, 0, 0]);
        if let Some(field) = icmp.get_mut(4..6) {
            field.copy_from_slice(&PROBE_ID.to_be_bytes());
        }
        if let Some(field) = icmp.get_mut(6..8) {
            field.copy_from_slice(&sequence.to_be_bytes());
        }
    }
    if let Ok(mut ip) = kith_packet::Ipv4Packet::new_checked(&mut packet[..]) {
        ip.fill_checksum();
    }
    if let Some(icmp) = packet.get_mut(20..)
        && let Ok(mut icmp) = kith_packet::IcmpPacket::new_checked(icmp)
    {
        icmp.fill_checksum_v4();
    }
    packet
}

/// The sequence number of `packet` if it answers one of our size probes.
fn echo_reply_sequence(packet: &[u8]) -> Option<u16> {
    let ip = kith_packet::Ipv4Packet::new_checked(packet).ok()?;
    let icmp = ip.payload();
    let kind = *icmp.first()?;
    let id = u16::from_be_bytes(icmp.get(4..6)?.try_into().ok()?);
    if ip.protocol() != 1 || kind != 0 || id != PROBE_ID {
        return None;
    }
    Some(u16::from_be_bytes(icmp.get(6..8)?.try_into().ok()?))
}

/// A packet's parts: the fragments it arrived in, or itself.
fn each<'a>(whole: &'a [u8], parts: Option<&'a [Vec<u8>]>) -> impl Iterator<Item = &'a [u8]> {
    core::iter::once(whole)
        .filter(move |_| parts.is_none())
        .chain(parts.into_iter().flatten().map(Vec::as_slice))
}

/// The member behind this device that `destination` belongs to, if any.
fn member_route(externals: &BTreeMap<u64, External>, destination: IpAddr) -> Option<(u64, PeerId)> {
    externals.iter().find_map(|(slot, external)| {
        let (_, id) = external.routes.lookup(destination)?;
        external
            .peers
            .get(id)
            .is_some_and(|entry| entry.member.is_some())
            .then_some((*slot, *id))
    })
}

/// Cryptokey routing: whether the peer may send from the packet's source address.
fn owns_source(entry: &PeerEntry, role: ExitRole, drops: &mut DropCounters, packet: &[u8]) -> bool {
    let Some(source) = packet::source(packet) else {
        count(&mut drops.malformed);
        return false;
    };
    if role == ExitRole::Current
        || entry
            .allowed_ips
            .iter()
            .any(|prefix| prefix.contains(source))
    {
        return true;
    }
    if role == ExitRole::Former {
        count(&mut drops.after_exit);
        return false;
    }
    count(&mut drops.source_not_allowed);
    debug!(
        network = %entry.network.short(),
        %source,
        destination = ?packet::destination(packet),
        "dropped a packet from an address its sender does not own"
    );
    false
}

/// Cryptokey routing (the peer may use this source address), then the network's filter.
fn admits(
    entry: &PeerEntry,
    role: ExitRole,
    filters: &mut BTreeMap<NetworkId, Filter>,
    drops: &mut DropCounters,
    packet: &[u8],
    now: Duration,
) -> bool {
    if !owns_source(entry, role, drops, packet) {
        return false;
    }
    if let Some(filter) = filters.get_mut(&entry.network) {
        filter.inbound(packet, now) == Verdict::Accept
    } else {
        count(&mut drops.policy);
        false
    }
}

impl Output {
    fn send(&mut self, peer: NodeId, endpoint: Option<Endpoint>, datagram: Option<&[u8]>) {
        let Some(datagram) = datagram else {
            return;
        };
        match endpoint {
            Some(Endpoint::Direct(destination)) => self.send_direct(destination, Some(datagram)),
            Some(Endpoint::Relay) => self.emit(Event::Relay {
                peer,
                datagram: datagram.to_vec(),
            }),
            None => count(&mut self.drops.no_endpoint),
        }
    }

    fn send_external(&mut self, endpoint: Option<SocketAddr>, datagram: Option<&[u8]>) {
        match endpoint {
            Some(destination) => self.send_direct(destination, datagram),
            None => count(&mut self.drops.no_endpoint),
        }
    }

    fn send_direct(&mut self, destination: SocketAddr, datagram: Option<&[u8]>) {
        if let Some(datagram) = datagram
            && self.udp.push(destination, datagram).is_err()
        {
            count(&mut self.drops.overflow);
        }
    }

    fn reply(&mut self, via: Via, datagram: Option<&[u8]>) {
        match via {
            Via::Udp(source) => self.send_direct(source, datagram),
            Via::Relay(peer) => self.send(peer, Some(Endpoint::Relay), datagram),
        }
    }

    fn session_event(&mut self, peer: NodeId, network: NetworkId, event: PeerEvent) {
        let event = match event {
            PeerEvent::SessionEstablished => {
                debug!(peer = %peer.short(), network = %network.short(), "WireGuard session established");
                Event::HandshakeCompleted { peer }
            }
            PeerEvent::SessionExpired => {
                debug!(peer = %peer.short(), network = %network.short(), "WireGuard session expired");
                Event::SessionExpired { peer }
            }
            _ => return,
        };
        self.emit(event);
    }

    /// A slow consumer loses events rather than stalling packets.
    fn emit(&mut self, event: Event) {
        if self.events.try_send(event).is_err() {
            count(&mut self.drops.overflow);
        }
    }
}

fn encode_address(address: SocketAddr, buffer: &mut [u8; 18]) -> &[u8] {
    let (ip, len) = match address.ip() {
        IpAddr::V4(ip) => (ip.to_ipv6_mapped().octets(), 4),
        IpAddr::V6(ip) => (ip.octets(), 16),
    };
    let ip = ip.get(16 - len..).unwrap_or_default();
    let port = address.port().to_be_bytes();
    for (slot, byte) in buffer.iter_mut().zip(ip.iter().chain(&port)) {
        *slot = *byte;
    }
    buffer.get(..len + 2).unwrap_or_default()
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => core::future::pending().await,
    }
}

struct OsRandom;

impl RandomSource for OsRandom {
    fn fill_bytes(&mut self, buffer: &mut [u8]) -> Result<(), WgError> {
        kith_crypto::fill_random(buffer).map_err(|_| WgError::RandomFailed)
    }
}

struct Clock {
    origin: Instant,
}

impl Clock {
    fn new() -> Self {
        Self {
            origin: read_clock().0,
        }
    }

    fn now(&self) -> Now {
        let (instant, wall) = read_clock();
        Now {
            monotonic: instant.saturating_duration_since(self.origin),
            wall: Tai64N::from_unix(wall.duration_since(UNIX_EPOCH).unwrap_or_default()),
        }
    }

    fn at(&self, monotonic: Duration) -> Option<Instant> {
        self.origin.checked_add(monotonic)
    }

    fn instant(&self, monotonic: Duration) -> Option<tokio::time::Instant> {
        self.at(monotonic).map(tokio::time::Instant::from_std)
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the dataplane driver reads the clock here, once per wakeup, for every core"
)]
fn read_clock() -> (Instant, SystemTime) {
    (Instant::now(), SystemTime::now())
}
