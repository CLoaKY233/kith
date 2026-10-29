use core::fmt;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;
use std::collections::BTreeMap;
use std::sync::Arc;

use kith_control::records::Relay;
use kith_crypto::{AgreementPublic, AgreementSecret, RandomError, SigningKey, VerifyingKey};
use kith_dataplane::Endpoint;
use kith_disco::{
    CallMeMaybe, Event, Message, PathReason, PathTarget, PeerPaths, RandomSource, Transmit,
};
use kith_identity::DeviceIdentity;
use kith_ipc::PathKind;
use kith_netmon::{Change, NetworkSnapshot, PortMapConfig, PortMapper};
use kith_relay::{Client, SendError, Status};
use kith_stun::{StunProbe, TransactionId, is_stun};
use kith_transport_quic::QuicEndpoint;
use kith_types::{IpPrefix, NodeId};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, warn};

use crate::stealth::{self, LadderIo, QuicLinks, StealthOptions, Tasks};
use crate::types::{Command, ConnectivityEvent, NodeError};

const STUN_INTERVAL: Duration = Duration::from_secs(30);
const MAX_PEER_CANDIDATES: usize = kith_disco::paths::MAX_CONTROL_CANDIDATES;
const MAX_OBSERVED: usize = kith_disco::paths::MAX_OBSERVED_CANDIDATES;

/// Something the loop must do on behalf of connectivity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Output {
    /// Send a STUN or disco datagram from the WireGuard socket.
    SendRaw {
        /// The destination.
        destination: SocketAddr,
        /// The datagram.
        datagram: Vec<u8>,
    },
    /// Feed a relayed WireGuard datagram into the dataplane.
    AcceptRelayed {
        /// The sending peer.
        peer: NodeId,
        /// The datagram.
        datagram: Vec<u8>,
    },
    /// Point a peer's WireGuard endpoint at a new path.
    PathChanged {
        /// The peer.
        peer: NodeId,
        /// The new endpoint.
        endpoint: Endpoint,
    },
    /// The largest packet the path to a peer carries changed; `None` is no known limit.
    PathMtu {
        /// The peer.
        peer: NodeId,
        /// The new limit.
        mtu: Option<u16>,
    },
}

struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&mut self, buffer: &mut [u8]) -> Result<(), RandomError> {
        kith_crypto::fill_random(buffer)
    }
}

struct DiscoSecret(AgreementSecret);

impl fmt::Debug for DiscoSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DiscoSecret(<redacted>)")
    }
}

#[derive(Debug)]
struct Peer {
    paths: PeerPaths,
    disco: [u8; 32],
    candidates: Vec<SocketAddr>,
}

#[derive(Debug)]
struct PeerLadder {
    address: String,
    direct: watch::Sender<bool>,
    _task: Tasks,
}

#[derive(Debug)]
struct RelayClient {
    address: String,
    client: Client,
}

/// Per-peer path selection, STUN, relay clients, port mapping and network changes.
#[derive(Debug)]
pub(crate) struct Connectivity {
    identity: Arc<DeviceIdentity>,
    commands: mpsc::Sender<Command>,
    peers: BTreeMap<NodeId, Peer>,
    outputs: Vec<Output>,
    epoch: Instant,
    local_port: u16,
    bound: IpAddr,
    secret: DiscoSecret,
    relay_key: Arc<SigningKey>,
    relays: BTreeMap<NodeId, RelayClient>,
    stun_servers: Vec<SocketAddr>,
    probes: BTreeMap<SocketAddr, StunProbe>,
    next_stun: Option<Instant>,
    interfaces: Vec<SocketAddr>,
    /// Addresses the config names for us, such as a server's `public_hostname`.
    configured: Vec<SocketAddr>,
    /// Every network's mesh ranges: an address in them exists only through a tunnel, so it
    /// is never a path to a peer, whoever reports it.
    mesh: Vec<IpPrefix>,
    reflexive: BTreeMap<SocketAddr, SocketAddr>,
    observed: Vec<SocketAddr>,
    mapped: Option<SocketAddr>,
    advertised: Vec<SocketAddr>,
    stealth: StealthOptions,
    /// The tunnel MTU, which every path is probed for.
    tunnel_mtu: u16,
    quic: Option<Arc<QuicEndpoint>>,
    links: QuicLinks,
    ladders: BTreeMap<NodeId, PeerLadder>,
    _tasks: Tasks,
}

impl Connectivity {
    /// Starts the netmon and port-mapping tasks, which report through `commands`.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the daemon treats this as fallible"
    )]
    pub(crate) fn start(
        identity: Arc<DeviceIdentity>,
        bound: SocketAddr,
        tun_name: Option<String>,
        commands: mpsc::Sender<Command>,
        stealth: StealthOptions,
        tunnel_mtu: u16,
    ) -> Result<Self, NodeError> {
        let tun_name = tun_name.unwrap_or_default();
        let local_port = bound.port();
        let bound = bound.ip();
        let interfaces = match NetworkSnapshot::capture(&tun_name) {
            Ok(snapshot) => interface_endpoints(Some(&snapshot), bound, local_port),
            Err(error) => {
                warn!(%error, "could not read local interfaces");
                interface_endpoints(None, bound, local_port)
            }
        };
        let tasks = Tasks(vec![
            spawn_netmon(tun_name, commands.clone()),
            spawn_port_mapper(local_port, commands.clone()),
        ]);
        let signing = identity.signing_key();
        let secret = DiscoSecret(signing.to_agreement_secret());
        let relay_key = Arc::new(SigningKey::from_seed(&signing.to_seed()));
        let quic = if stealth.wants_quic() {
            stealth::client_endpoint(&relay_key)
        } else {
            None
        };
        let mut connectivity = Self {
            identity,
            commands,
            peers: BTreeMap::new(),
            outputs: Vec::new(),
            epoch: Instant::now(),
            local_port,
            bound,
            secret,
            relay_key,
            relays: BTreeMap::new(),
            stun_servers: Vec::new(),
            probes: BTreeMap::new(),
            next_stun: None,
            interfaces,
            configured: Vec::new(),
            mesh: Vec::new(),
            reflexive: BTreeMap::new(),
            observed: Vec::new(),
            mapped: None,
            advertised: Vec::new(),
            stealth,
            tunnel_mtu,
            quic,
            links: QuicLinks::default(),
            ladders: BTreeMap::new(),
            _tasks: tasks,
        };
        connectivity.refresh_advertised();
        Ok(connectivity)
    }

    /// Replaces the set of mesh peers; new ones start probing, removed ones are dropped.
    pub(crate) fn set_peers(&mut self, peers: &[NodeId], now: Instant) {
        let local = self.identity.node_id();
        self.peers
            .retain(|node, _| peers.contains(node) && *node != local);
        let at = self.at(now);
        let relay_available = !self.relays.is_empty();
        for node in peers {
            if *node == local || self.peers.contains_key(node) {
                continue;
            }
            let disco = match VerifyingKey::from_node_id(*node) {
                Ok(key) => key.to_agreement_public().to_bytes(),
                Err(error) => {
                    warn!(peer = %node, %error, "peer identity is not a valid key");
                    continue;
                }
            };
            let mut paths = match PeerPaths::new(local, *node, &mut OsRandom) {
                Ok(paths) => paths,
                Err(error) => {
                    warn!(peer = %node, %error, "could not seed disco state");
                    continue;
                }
            };
            paths.set_local_endpoints(&self.advertised);
            paths.set_tunnel_mtu(self.tunnel_mtu);
            paths.set_relay_available(at, relay_available);
            paths.request_hole_punch(at);
            self.peers.insert(
                *node,
                Peer {
                    paths,
                    disco,
                    candidates: Vec::new(),
                },
            );
            self.pump(*node, now);
        }
        self.sync_ladders();
    }

    /// The QUIC links carrying WireGuard datagrams, shared with server mode.
    pub(crate) fn quic_links(&self) -> QuicLinks {
        self.links.clone()
    }

    fn sync_ladders(&mut self) {
        let Some(endpoint) = &self.quic else {
            return;
        };
        let peers = &self.peers;
        let relays = &self.relays;
        self.ladders.retain(|node, ladder| {
            peers.contains_key(node)
                && relays
                    .get(node)
                    .is_some_and(|relay| relay.address == ladder.address)
        });
        for (node, relay) in relays {
            let Some(peer) = peers.get(node) else {
                continue;
            };
            if self.ladders.contains_key(node) {
                continue;
            }
            let direct = matches!(peer.paths.best_path(), Some(PathTarget::Direct(_)));
            let (sender, receiver) = watch::channel(direct);
            let Some(task) = stealth::spawn_peer_ladder(
                *node,
                &relay.address,
                &self.stealth,
                receiver,
                LadderIo {
                    endpoint: Arc::clone(endpoint),
                    links: self.links.clone(),
                    commands: self.commands.clone(),
                },
            ) else {
                continue;
            };
            debug!(peer = %node, "transport ladder started");
            self.ladders.insert(
                *node,
                PeerLadder {
                    address: relay.address.clone(),
                    direct: sender,
                    _task: Tasks(vec![task]),
                },
            );
        }
    }

    /// Adds candidate addresses for a peer from gossip or its control connection.
    /// Sets the mesh ranges whose addresses are never paths.
    pub(crate) fn set_mesh_ranges(&mut self, ranges: Vec<IpPrefix>) {
        if self.mesh != ranges {
            self.mesh = ranges;
            for state in self.peers.values_mut() {
                state.candidates.retain(|addr| !in_ranges(&self.mesh, addr));
            }
            self.refresh_advertised();
        }
    }

    fn in_mesh(&self, addr: &SocketAddr) -> bool {
        in_ranges(&self.mesh, addr)
    }

    pub(crate) fn add_candidates(&mut self, peer: NodeId, addrs: &[SocketAddr], now: Instant) {
        let addrs: Vec<SocketAddr> = addrs.iter().copied().filter(|a| !self.in_mesh(a)).collect();
        let addrs = addrs.as_slice();
        let at = self.at(now);
        let Some(state) = self.peers.get_mut(&peer) else {
            return;
        };
        let before = state.candidates.clone();
        for addr in addrs.iter().rev() {
            state.candidates.retain(|known| known != addr);
            state.candidates.insert(0, *addr);
        }
        state.candidates.truncate(MAX_PEER_CANDIDATES);
        if state.candidates != before {
            state.paths.set_candidates(at, &state.candidates);
            state.paths.request_hole_punch(at);
            self.pump(peer, now);
        }
    }

    /// Connects to every advertised relay and drops clients for withdrawn ones. Our own
    /// relay is reached on `own_relay`, loopback, so peers can relay to us as to anyone.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the daemon treats this as fallible"
    )]
    pub(crate) fn set_relays(
        &mut self,
        relays: &[Relay],
        own_relay: Option<SocketAddr>,
        resolved: &BTreeMap<String, Vec<SocketAddr>>,
    ) -> Result<(), NodeError> {
        let local = self.identity.node_id();
        let wanted: BTreeMap<NodeId, String> = relays
            .iter()
            .filter_map(|relay| {
                let address = if relay.node() == local {
                    own_relay?.to_string()
                } else {
                    relay.address().to_owned()
                };
                Some((relay.node(), address))
            })
            .collect();
        self.relays.retain(|node, client| {
            wanted
                .get(node)
                .is_some_and(|address| *address == client.address)
        });
        for (node, address) in &wanted {
            if self.relays.contains_key(node) {
                continue;
            }
            let (client, incoming) = stealth::relay_client(
                address.clone(),
                *node,
                Arc::clone(&self.relay_key),
                &self.stealth,
            );
            tokio::spawn(forward_relayed(incoming, self.commands.clone()));
            debug!(relay = %node, %address, "relay client started");
            self.relays.insert(
                *node,
                RelayClient {
                    address: address.clone(),
                    client,
                },
            );
        }
        let stun_servers: Vec<SocketAddr> = relays
            .iter()
            .filter(|relay| relay.node() != local)
            .flat_map(|relay| match relay.address().parse::<SocketAddr>() {
                Ok(addr) => vec![addr],
                Err(_) => resolved.get(relay.address()).cloned().unwrap_or_default(),
            })
            .collect();
        if stun_servers != self.stun_servers {
            self.stun_servers = stun_servers;
            self.probes.clear();
            self.reflexive
                .retain(|server, _| self.stun_servers.contains(server));
            self.next_stun = Some(Instant::now());
            self.refresh_advertised();
        }
        let now = Instant::now();
        let at = self.at(now);
        let available = !self.relays.is_empty();
        let nodes: Vec<NodeId> = self.peers.keys().copied().collect();
        for node in nodes {
            if let Some(peer) = self.peers.get_mut(&node) {
                peer.paths.set_relay_available(at, available);
            }
            self.pump(node, now);
        }
        self.sync_ladders();
        Ok(())
    }

    /// Handles a STUN or disco datagram from the dataplane's control event.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the daemon treats this as fallible"
    )]
    pub(crate) fn handle_datagram(
        &mut self,
        source: SocketAddr,
        datagram: &[u8],
        now: Instant,
    ) -> Result<(), NodeError> {
        if self.in_mesh(&source) {
            debug!(%source, "ignoring disco or STUN that came through a tunnel");
            return Ok(());
        }
        if is_stun(datagram) {
            self.handle_stun(source, datagram);
            return Ok(());
        }
        let Some((peer, message)) = self.open(datagram) else {
            return Ok(());
        };
        let at = self.at(now);
        let Some(state) = self.peers.get_mut(&peer) else {
            return Ok(());
        };
        match message {
            Message::Ping(ping) => {
                if let Some(reply) = state.paths.handle_ping(at, source, &ping) {
                    self.transmit(peer, &reply);
                }
            }
            Message::Pong(pong) => {
                state.paths.handle_pong(at, source, &pong);
            }
            Message::CallMeMaybe(call) => {
                if let Some(call) = without_ranges(&self.mesh, &call) {
                    state.paths.handle_call_me_maybe(at, &call);
                }
            }
        }
        self.pump(peer, now);
        Ok(())
    }

    /// Sends a WireGuard datagram for `peer` over its QUIC link, else through a connected relay.
    pub(crate) fn send_relayed(
        &mut self,
        peer: NodeId,
        datagram: Vec<u8>,
    ) -> Result<(), NodeError> {
        if self.links.send(peer, &datagram) {
            return Ok(());
        }
        let mut clients: Vec<&RelayClient> = self.relays.values().collect();
        clients.sort_by_key(|relay| relay.client.status() != Status::Connected);
        let Some(relay) = clients
            .into_iter()
            .find(|relay| !matches!(relay.client.status(), Status::Rejected(_)))
        else {
            return Err(NodeError::NotFound(format!("no relay to reach {peer}")));
        };
        match relay.client.send(peer, datagram) {
            Ok(()) | Err(SendError::Full) => Ok(()),
            Err(SendError::TooLarge(len)) => Err(NodeError::InvalidArgument(format!(
                "datagram of {len} bytes exceeds the relay limit"
            ))),
            Err(error) => Err(NodeError::NotFound(format!("relay unusable: {error}"))),
        }
    }

    /// Handles a datagram a relay delivered, disco or WireGuard.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the daemon treats this as fallible"
    )]
    pub(crate) fn handle_relayed(
        &mut self,
        from: NodeId,
        datagram: Vec<u8>,
        now: Instant,
    ) -> Result<(), NodeError> {
        if !kith_disco::is_disco(&datagram) {
            self.outputs.push(Output::AcceptRelayed {
                peer: from,
                datagram,
            });
            return Ok(());
        }
        let Some((sender, message)) = self.open(&datagram) else {
            return Ok(());
        };
        if sender != from {
            debug!(relay_sender = %from, disco_sender = %sender, "relayed disco sender mismatch");
            return Ok(());
        }
        if let Message::CallMeMaybe(call) = message {
            let at = self.at(now);
            if let Some(state) = self.peers.get_mut(&from)
                && let Some(call) = without_ranges(&self.mesh, &call)
            {
                state.paths.handle_call_me_maybe(at, &call);
                self.pump(from, now);
            }
        }
        Ok(())
    }

    /// Re-gathers local endpoints and re-probes every peer after a host network change.
    pub(crate) fn handle_network_change(&mut self, change: &Change, now: Instant) {
        self.interfaces = interface_endpoints(Some(&change.new), self.bound, self.local_port);
        self.reflexive.clear();
        self.observed.clear();
        self.probes.clear();
        self.next_stun = (!self.stun_servers.is_empty()).then_some(now);
        self.refresh_advertised();
        let at = self.at(now);
        let nodes: Vec<NodeId> = self.peers.keys().copied().collect();
        for node in nodes {
            if let Some(peer) = self.peers.get_mut(&node) {
                peer.paths.request_hole_punch(at);
            }
            self.pump(node, now);
        }
    }

    /// Records the router's current port mapping as a local candidate.
    pub(crate) fn set_port_mapping(&mut self, mapped: Option<SocketAddr>) {
        if self.mapped != mapped {
            self.mapped = mapped;
            self.refresh_advertised();
        }
    }

    /// Notes that packets are flowing with a peer, or its WireGuard session just came up,
    /// so its paths keep being checked.
    pub(crate) fn note_activity(&mut self, peer: NodeId, now: Instant) {
        let at = self.at(now);
        if let Some(state) = self.peers.get_mut(&peer) {
            state.paths.note_activity(at);
            self.pump(peer, now);
        }
    }

    /// Advances every timer that is due.
    pub(crate) fn handle_timeout(&mut self, now: Instant) {
        if self.next_stun.is_some_and(|due| due <= now) {
            self.start_stun_round(now);
        }
        let at = self.at(now);
        let mut requests = Vec::new();
        self.probes.retain(|server, probe| {
            if let Some(request) = probe.poll_transmit(at) {
                requests.push(Output::SendRaw {
                    destination: *server,
                    datagram: request.to_vec(),
                });
            }
            !probe.is_timed_out()
        });
        self.outputs.extend(requests);
        let nodes: Vec<NodeId> = self.peers.keys().copied().collect();
        for node in nodes {
            if let Some(peer) = self.peers.get_mut(&node) {
                peer.paths.handle_timeout(at);
            }
            self.pump(node, now);
        }
    }

    /// The next instant [`Connectivity::handle_timeout`] must run.
    pub(crate) fn poll_timeout(&self) -> Option<Instant> {
        let peers = self
            .peers
            .values()
            .filter_map(|peer| peer.paths.poll_timeout());
        let probes = self.probes.values().filter_map(StunProbe::poll_timeout);
        peers
            .chain(probes)
            .filter_map(|at| self.epoch.checked_add(at))
            .chain(self.next_stun)
            .min()
    }

    /// Takes the queued outputs.
    pub(crate) fn drain_outputs(&mut self) -> Vec<Output> {
        core::mem::take(&mut self.outputs)
    }

    /// Our endpoints worth gossiping: local, STUN-observed and port-mapped.
    pub(crate) fn local_endpoints(&self) -> Vec<SocketAddr> {
        self.advertised.clone()
    }

    /// The path currently used for a peer.
    /// Why traffic to `peer` takes its current path.
    pub(crate) fn path_reason(&self, peer: &NodeId) -> Option<String> {
        let state = self.peers.get(peer)?;
        let reason = state.paths.explain();
        Some(match reason {
            PathReason::Direct { .. } if self.stealth.stealth_only() => {
                "relay: direct UDP is turned off (transports.stealth_only)".to_owned()
            }
            PathReason::Direct { .. } | PathReason::NoDirectAnswer { .. } => reason.to_string(),
        })
    }

    /// The path traffic to `peer` takes now, as the dataplane sees it.
    pub(crate) fn path_kind(&self, peer: &NodeId) -> Option<PathKind> {
        self.peers
            .get(peer)
            .and_then(|state| state.paths.best_path())
            .map(|target| match target {
                PathTarget::Direct(_) if !self.stealth.stealth_only() => PathKind::Direct,
                PathTarget::Direct(_) | PathTarget::Relay => PathKind::Relay,
            })
    }

    fn at(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.epoch)
    }

    fn open(&self, datagram: &[u8]) -> Option<(NodeId, Message)> {
        let (sender, message) = match kith_disco::open(datagram, &self.secret.0) {
            Ok(opened) => opened,
            Err(error) => {
                debug!(%error, "dropping undecodable disco packet");
                return None;
            }
        };
        let key = sender.to_bytes();
        let peer = self
            .peers
            .iter()
            .find(|(_, peer)| peer.disco == key)
            .map(|(node, _)| *node);
        if peer.is_none() {
            debug!("dropping disco packet from an unknown key");
        }
        peer.map(|node| (node, message))
    }

    fn pump(&mut self, node: NodeId, now: Instant) {
        let at = self.at(now);
        let mut transmits = Vec::new();
        let mut events = Vec::new();
        let Some(peer) = self.peers.get_mut(&node) else {
            return;
        };
        while let Some(transmit) = peer.paths.poll_transmit(at) {
            transmits.push(transmit);
        }
        while let Some(event) = peer.paths.poll_event() {
            events.push(event);
        }
        for transmit in transmits {
            self.transmit(node, &transmit);
        }
        let mut observed_changed = false;
        for event in events {
            match event {
                Event::PathChanged(target) => {
                    let direct = match target {
                        Some(PathTarget::Direct(addr)) => Some(addr),
                        Some(PathTarget::Relay) | None => None,
                    };
                    if let Some(ladder) = self.ladders.get(&node) {
                        ladder.direct.send_replace(direct.is_some());
                    }
                    let endpoint = match direct {
                        Some(addr) if !self.stealth.stealth_only() => Endpoint::Direct(addr),
                        Some(_) | None => Endpoint::Relay,
                    };
                    debug!(peer = %node, ?endpoint, "path changed");
                    self.outputs.push(Output::PathChanged {
                        peer: node,
                        endpoint,
                    });
                }
                Event::EndpointObserved(addr)
                    if !self.observed.contains(&addr) && !in_ranges(&self.mesh, &addr) =>
                {
                    self.observed.insert(0, addr);
                    self.observed.truncate(MAX_OBSERVED);
                    observed_changed = true;
                }
                Event::PathMtuChanged(mtu) => {
                    debug!(peer = %node, ?mtu, "path size changed");
                    self.outputs.push(Output::PathMtu { peer: node, mtu });
                }
                Event::EndpointObserved(_) | _ => {}
            }
        }
        if observed_changed {
            self.refresh_advertised();
        }
    }

    fn transmit(&mut self, node: NodeId, transmit: &Transmit) {
        let Some(peer) = self.peers.get(&node) else {
            return;
        };
        let key = AgreementPublic::from_bytes(peer.disco);
        let datagram =
            match kith_disco::seal(&transmit.message, &self.secret.0, &key, &mut OsRandom) {
                Ok(datagram) => datagram,
                Err(error) => {
                    warn!(peer = %node, %error, "could not seal disco message");
                    return;
                }
            };
        match transmit.to {
            PathTarget::Direct(destination) => self.outputs.push(Output::SendRaw {
                destination,
                datagram,
            }),
            PathTarget::Relay => {
                if let Err(error) = self.send_relayed(node, datagram) {
                    debug!(peer = %node, %error, "disco message not relayed");
                }
            }
        }
    }

    fn handle_stun(&mut self, source: SocketAddr, datagram: &[u8]) {
        let Some(probe) = self.probes.get_mut(&source) else {
            return;
        };
        if let Some(reflexive) = probe.handle_response(datagram) {
            self.probes.remove(&source);
            if self.reflexive.insert(source, reflexive) != Some(reflexive) {
                debug!(server = %source, %reflexive, "STUN reflexive address");
                self.refresh_advertised();
            }
        }
    }

    fn start_stun_round(&mut self, now: Instant) {
        self.next_stun = now.checked_add(STUN_INTERVAL);
        let at = self.at(now);
        for server in &self.stun_servers {
            let mut txid = [0_u8; 12];
            if let Err(error) = kith_crypto::fill_random(&mut txid) {
                warn!(%error, "could not start STUN probe");
                return;
            }
            self.probes
                .insert(*server, StunProbe::new(TransactionId(txid), at));
        }
    }

    /// Sets the addresses the config names for us; they lead the endpoints we offer.
    pub(crate) fn set_configured_endpoints(&mut self, endpoints: Vec<SocketAddr>) {
        if self.configured != endpoints {
            self.configured = endpoints;
            self.refresh_advertised();
        }
    }

    fn refresh_advertised(&mut self) {
        let mut advertised: Vec<SocketAddr> = Vec::new();
        let sources = self
            .configured
            .iter()
            .chain(&self.mapped)
            .chain(self.reflexive.values())
            .chain(&self.observed)
            .chain(&self.interfaces);
        for addr in sources.filter(|addr| !in_ranges(&self.mesh, addr)) {
            if !advertised.contains(addr) {
                advertised.push(*addr);
            }
        }
        advertised.truncate(kith_disco::paths::MAX_LOCAL_ENDPOINTS);
        if advertised == self.advertised {
            return;
        }
        debug!(endpoints = ?advertised, "local endpoints changed");
        for peer in self.peers.values_mut() {
            peer.paths.set_local_endpoints(&advertised);
        }
        self.advertised = advertised;
    }
}

fn interface_endpoints(
    snapshot: Option<&NetworkSnapshot>,
    bound: IpAddr,
    port: u16,
) -> Vec<SocketAddr> {
    if !bound.is_unspecified() {
        return vec![SocketAddr::new(bound, port)];
    }
    snapshot
        .into_iter()
        .flat_map(NetworkSnapshot::addresses)
        .filter(|interface| !is_tunnel(&interface.interface))
        .map(|interface| interface.address)
        .filter(|address| match address {
            IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_unspecified() && !v4.is_link_local(),
            IpAddr::V6(v6) => {
                !v6.is_loopback() && !v6.is_unspecified() && !v6.is_unicast_link_local()
            }
        })
        .map(|address| SocketAddr::new(address, port))
        .collect()
}

fn in_ranges(ranges: &[IpPrefix], addr: &SocketAddr) -> bool {
    ranges.iter().any(|range| range.contains(addr.ip()))
}

/// `call` without endpoints in `ranges`, or `None` when none are left.
fn without_ranges(ranges: &[IpPrefix], call: &CallMeMaybe) -> Option<CallMeMaybe> {
    let kept: Vec<SocketAddr> = call
        .endpoints()
        .iter()
        .copied()
        .filter(|addr| !in_ranges(ranges, addr))
        .collect();
    if kept.is_empty() {
        return None;
    }
    CallMeMaybe::new(kept)
}

/// Tunnel interfaces, kith's own and other VPNs': their addresses only work through the
/// tunnel, so offering them as endpoints sends kith's own traffic through a tunnel.
fn is_tunnel(name: &str) -> bool {
    ["utun", "tun", "wg", "tailscale", "zt"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

async fn forward_relayed(mut incoming: kith_relay::Incoming, commands: mpsc::Sender<Command>) {
    while let Some((from, datagram)) = incoming.recv().await {
        let event = ConnectivityEvent::Relayed { from, datagram };
        if commands.send(Command::Connectivity(event)).await.is_err() {
            return;
        }
    }
}

fn spawn_netmon(tun_name: String, commands: mpsc::Sender<Command>) -> JoinHandle<()> {
    let mut changes = kith_netmon::spawn_monitor(tun_name);
    tokio::spawn(async move {
        while let Some(change) = changes.recv().await {
            let event = ConnectivityEvent::NetworkChanged(Box::new(change));
            if commands.send(Command::Connectivity(event)).await.is_err() {
                return;
            }
        }
    })
}

fn spawn_port_mapper(local_port: u16, commands: mpsc::Sender<Command>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut mapper = PortMapper::spawn(PortMapConfig::new(local_port));
        let mut current = None;
        loop {
            let mapping = mapper.changed().await.map(|mapping| mapping.external);
            if mapping == current {
                if mapping.is_none() {
                    tokio::time::sleep(STUN_INTERVAL).await;
                }
                continue;
            }
            current = mapping;
            let event = ConnectivityEvent::PortMapped(mapping);
            if commands.send(Command::Connectivity(event)).await.is_err() {
                mapper.shutdown().await;
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use kith_netmon::{InterfaceAddress, NetworkSnapshot};

    use kith_disco::CallMeMaybe;
    use kith_types::IpPrefix;

    use super::{interface_endpoints, without_ranges};

    #[test]
    fn tunnel_addresses_are_never_offered_as_endpoints() {
        let address = |interface: &str, ip: &str| InterfaceAddress {
            interface: interface.to_owned(),
            address: ip.parse().unwrap(),
            prefix_length: 24,
        };
        let snapshot = NetworkSnapshot::new(
            vec![
                address("en0", "192.168.1.20"),
                address("docker0", "172.17.0.1"),
                address("utun41", "100.88.250.27"),
                address("tun0", "100.70.0.2"),
                address("tailscale0", "100.101.1.1"),
                address("wg0", "10.8.0.2"),
            ],
            None,
        );
        let found = interface_endpoints(Some(&snapshot), "0.0.0.0".parse().unwrap(), 41641);
        let expected: Vec<std::net::SocketAddr> = vec![
            "172.17.0.1:41641".parse().unwrap(),
            "192.168.1.20:41641".parse().unwrap(),
        ];
        assert_eq!(found, expected);
    }

    #[test]
    fn mesh_addresses_are_never_taken_as_paths() {
        let mesh: Vec<IpPrefix> = vec![
            "100.64.0.0/10".parse().unwrap(),
            "fd7a:115c:a1e0::/48".parse().unwrap(),
        ];
        let addr = |text: &str| text.parse::<std::net::SocketAddr>().unwrap();
        let call = CallMeMaybe::new(vec![
            addr("3.110.233.43:41641"),
            addr("100.84.209.2:41641"),
            addr("[fd7a:115c:a1e0:1::2]:41641"),
        ])
        .unwrap();
        let kept = without_ranges(&mesh, &call).unwrap();
        assert_eq!(kept.endpoints(), [addr("3.110.233.43:41641")]);

        let only_mesh = CallMeMaybe::new(vec![addr("100.84.209.2:41641")]).unwrap();
        assert!(without_ranges(&mesh, &only_mesh).is_none());
    }
}
