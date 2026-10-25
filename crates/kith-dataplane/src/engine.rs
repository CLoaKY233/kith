//! The single-task packet loop.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kith_crypto::{SigningKey, VerifyingKey};
use kith_filter::{Filter, Verdict};
use kith_router::Router;
use kith_transport_udp::{Kind, MAX_SLOT_LEN, RecvBatch, SendBatch, UdpLink, classify};
use kith_tun::{MAX_PACKET_CAPACITY, PacketBatch};
use kith_types::{IpPrefix, NetworkId, NodeId};
use kith_wg::{
    CookieChecker, Decapsulated, Device, Encapsulated, Now, PeerConfig, PeerId, RandomSource,
    Tai64N, Transmit, WgError, required_output_len,
};
use tokio::sync::mpsc;

use crate::api::{Command, DataplaneError, DataplaneHandle, Endpoint, Event, PeerSpec, channels};
use crate::packet;
use crate::tun_device::TunDevice;

/// Handshakes per second answered before cookies are demanded.
const HANDSHAKES_PER_SECOND: u32 = 1000;

/// A peer whose timer is due but has nothing to send must not spin the loop.
const MIN_TIMER_DELAY: Duration = Duration::from_millis(1);

pub(crate) struct Engine<T> {
    io: Io<T>,
    state: State,
    commands: mpsc::Receiver<Command>,
    clock: Clock,
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
    tun_out: PacketBatch,
    out: Output,
    scratch: Vec<u8>,
}

struct PeerEntry {
    id: PeerId,
    network: NetworkId,
    allowed_ips: Vec<IpPrefix>,
    endpoint: Option<Endpoint>,
    keepalive: Option<u16>,
    established: bool,
}

struct Output {
    udp: SendBatch,
    events: mpsc::Sender<Event>,
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
        let secret = signing_key.to_agreement_secret().to_bytes();
        let device = Device::new(
            *secret,
            CookieChecker::new(HANDSHAKES_PER_SECOND),
            Box::new(OsRandom),
        );
        let engine = Self {
            state: State {
                device,
                router: Router::new(),
                filters: BTreeMap::new(),
                peers: BTreeMap::new(),
                by_index: BTreeMap::new(),
                tun_out: tun.new_batch()?,
                out: Output {
                    udp: SendBatch::with_capacity(kith_tun::BATCH_SIZE, 256 * 1024),
                    events,
                },
                scratch: vec![0; required_output_len(MAX_PACKET_CAPACITY).max(MAX_SLOT_LEN)],
            },
            io: Io {
                tun_in: tun.new_batch()?,
                udp_in: RecvBatch::compact(&link),
                tun,
                link,
            },
            commands,
            clock: Clock::new(),
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
                Wake::Command(command) => self.state.command(now, command),
                Wake::Tun => self.state.outbound(now, &self.io.tun_in),
                Wake::Udp => self.inbound(now).await,
                Wake::Timer => {}
            }
            deadline = self.state.poll_peers(now);
            self.flush().await;
        }
    }

    async fn inbound(&mut self, now: Now) {
        for datagram in self.io.udp_in.datagrams() {
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
                Kind::Unknown => {}
            }
        }
    }

    /// UDP and TUN writes are best effort, like the networks behind them.
    async fn flush(&mut self) {
        if !self.state.out.udp.is_empty() {
            self.io.link.send_batch(&self.state.out.udp).await.ok();
            self.state.out.udp.clear();
        }
        if !self.state.tun_out.is_empty() {
            self.io.tun.send_batch(&mut self.state.tun_out).await.ok();
        }
    }
}

impl State {
    fn command(&mut self, now: Now, command: Command) {
        match command {
            Command::SetPeers(peers, reply) => {
                reply.send(self.set_peers(peers)).ok();
            }
            Command::SetRoutes {
                network,
                mesh,
                routes,
                reply,
            } => {
                let result = self.router.set_network(network, mesh, routes);
                reply.send(result.map_err(DataplaneError::from)).ok();
            }
            Command::SetExit(exit, reply) => {
                reply.send(self.set_exit(exit)).ok();
            }
            Command::SetFilter(network, policy) => match self.filters.get_mut(&network) {
                Some(filter) => filter.set_policy(policy),
                None => {
                    self.filters.insert(network, Filter::new(policy));
                }
            },
            Command::SendRaw(destination, datagram) => {
                self.out.udp.push(destination, &datagram).ok();
            }
            Command::AcceptRelayed(peer, datagram) => {
                self.receive(now, Via::Relay(peer), &datagram);
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
            if let Some(entry) = self.peers.remove(&node) {
                self.device.remove_peer(entry.id);
                self.by_index.remove(&entry.id);
            }
        }
        for (node, (spec, key)) in wanted {
            if let Some(entry) = self.peers.get_mut(&node) {
                entry.network = spec.network;
                entry.allowed_ips = spec.allowed_ips;
                if spec.endpoint.is_some() {
                    entry.endpoint = spec.endpoint;
                }
                continue;
            }
            let mut config = PeerConfig::new(key);
            if let Some(seconds) = spec.persistent_keepalive {
                config = config.with_persistent_keepalive(seconds);
            }
            let id = self.device.add_peer(&config, Box::new(OsRandom))?;
            self.by_index.insert(id, node);
            self.peers.insert(
                node,
                PeerEntry {
                    id,
                    network: spec.network,
                    allowed_ips: spec.allowed_ips,
                    endpoint: spec.endpoint,
                    keepalive: spec.persistent_keepalive,
                    established: false,
                },
            );
        }
        Ok(())
    }

    fn set_exit(&mut self, exit: Option<(NetworkId, NodeId)>) -> Result<(), DataplaneError> {
        match exit {
            Some((network, peer)) => {
                self.router.set_exit_peer(network, Some(peer))?;
                self.router.set_active_exit(Some(network))?;
            }
            None => self.router.set_active_exit(None)?,
        }
        Ok(())
    }

    fn outbound(&mut self, now: Now, batch: &PacketBatch) {
        for packet in batch.iter() {
            let Some(route) = packet::destination(packet).and_then(|dst| self.router.route(dst))
            else {
                continue;
            };
            if let Some(filter) = self.filters.get_mut(&route.network)
                && filter.outbound(packet, now.monotonic) != Verdict::Accept
            {
                continue;
            }
            let Some(entry) = self.peers.get(&route.peer) else {
                continue;
            };
            let Some(peer) = self.device.peer_mut(entry.id) else {
                continue;
            };
            let Some(out) = self.scratch.get_mut(..required_output_len(packet.len())) else {
                continue;
            };
            if let Ok(Encapsulated::Transport(len) | Encapsulated::HandshakeInitiation(len)) =
                peer.encapsulate(now, packet, out)
            {
                self.out.send(route.peer, entry.endpoint, out.get(..len));
            }
        }
    }

    fn receive(&mut self, now: Now, via: Via, datagram: &[u8]) {
        let mut address = [0; 18];
        let source = match &via {
            Via::Udp(source) => encode_address(*source, &mut address),
            Via::Relay(peer) => peer.as_bytes().as_slice(),
        };
        let Ok(output) = self
            .device
            .decapsulate(now, source, datagram, &mut self.scratch)
        else {
            return;
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
            entry.endpoint = Some(Endpoint::Direct(source));
        }
        match output.result {
            Decapsulated::Packet(len) => {
                if let Some(packet) = self.scratch.get(..len)
                    && admits(entry, &mut self.filters, packet, now.monotonic)
                {
                    self.tun_out.push(packet).ok();
                }
            }
            Decapsulated::Reply(len) => self.out.reply(via, self.scratch.get(..len)),
            Decapsulated::Nothing => {}
        }
        if let Some(peer) = self.device.peer_mut(id) {
            self.out
                .note_session(node, entry, peer.has_session(now.monotonic));
        }
    }

    /// Drives due WireGuard timers and returns when the next one is due.
    fn poll_peers(&mut self, now: Now) -> Option<Duration> {
        let mut next: Option<Duration> = None;
        for (id, peer) in self.device.peers_mut() {
            if peer.poll_timeout().is_some_and(|due| due <= now.monotonic)
                && let Some(node) = self.by_index.get(&id)
                && let Some(entry) = self.peers.get_mut(node)
            {
                while let Ok(Transmit::Send(len)) = peer.poll_transmit(now, &mut self.scratch) {
                    self.out
                        .send(*node, entry.endpoint, self.scratch.get(..len));
                }
                self.out
                    .note_session(*node, entry, peer.has_session(now.monotonic));
            }
            if let Some(due) = peer.poll_timeout() {
                next = Some(next.map_or(due, |next| next.min(due)));
            }
        }
        next.map(|due| due.max(now.monotonic.saturating_add(MIN_TIMER_DELAY)))
    }
}

/// Cryptokey routing (the peer may use this source address), then the network's filter.
fn admits(
    entry: &PeerEntry,
    filters: &mut BTreeMap<NetworkId, Filter>,
    packet: &[u8],
    now: Duration,
) -> bool {
    let Some(source) = packet::source(packet) else {
        return false;
    };
    entry
        .allowed_ips
        .iter()
        .any(|prefix| prefix.contains(source))
        && filters
            .get_mut(&entry.network)
            .is_some_and(|filter| filter.inbound(packet, now) == Verdict::Accept)
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
            None => {}
        }
    }

    fn send_direct(&mut self, destination: SocketAddr, datagram: Option<&[u8]>) {
        if let Some(datagram) = datagram {
            self.udp.push(destination, datagram).ok();
        }
    }

    fn reply(&mut self, via: Via, datagram: Option<&[u8]>) {
        match via {
            Via::Udp(source) => self.send_direct(source, datagram),
            Via::Relay(peer) => self.send(peer, Some(Endpoint::Relay), datagram),
        }
    }

    fn note_session(&mut self, peer: NodeId, entry: &mut PeerEntry, has_session: bool) {
        if has_session && !entry.established {
            self.emit(Event::HandshakeCompleted { peer });
        }
        entry.established = has_session;
    }

    /// A slow consumer loses events rather than stalling packets.
    fn emit(&self, event: Event) {
        self.events.try_send(event).ok();
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

    fn instant(&self, monotonic: Duration) -> Option<tokio::time::Instant> {
        self.origin
            .checked_add(monotonic)
            .map(tokio::time::Instant::from_std)
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the dataplane driver reads the clock here, once per wakeup, for every core"
)]
fn read_clock() -> (Instant, SystemTime) {
    (Instant::now(), SystemTime::now())
}
