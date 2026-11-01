use core::net::SocketAddr;
use core::time::Duration;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use kith_channel::{Channel, ChannelClient, ChannelError, ChannelHandler, ChannelServer, Presence};
use kith_control::NetMap;
use kith_control::records::Relay;
use kith_identity::{Bootstrap, BootstrapAddr, DeviceIdentity, InviteToken};
use kith_ledger::{AdmissionRequest, JoinProof, Ledger};
use kith_types::{NetworkId, NodeId, Timestamp};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::admissions::Admissions;
use crate::network::{LedgerRegistry, sponsor_join};
use crate::types::{ChannelEvent, Command, NodeError};

/// Most direct peers a node keeps a control connection to, besides relays.
pub(crate) const MAX_DIRECT_SESSIONS: usize = 8;

/// How often every session re-syncs as a safety net.
pub(crate) const RESYNC_INTERVAL: Duration = Duration::from_secs(60);

const BACKOFF_MIN: Duration = Duration::from_secs(1);

const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// A peer to keep a control connection and sync session with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncTarget {
    /// The peer, pinned during the TLS handshake.
    pub(crate) node: NodeId,
    /// Addresses to dial, best first.
    pub(crate) addrs: Vec<SocketAddr>,
}

/// The `kith_channel::ChannelHandler`: reads the registry, sponsors joins, reports to the loop.
#[derive(Debug)]
pub(crate) struct NodeHandler {
    ledgers: LedgerRegistry,
    identity: Arc<DeviceIdentity>,
    commands: mpsc::Sender<Command>,
    admissions: SharedAdmissions,
}

/// Join requests waiting for an admin, shared by the handler and the loop.
pub(crate) type SharedAdmissions = Arc<Mutex<Admissions>>;

/// Locks the admissions; they hold no invariant a panicking holder could break.
pub(crate) fn lock_admissions(
    admissions: &SharedAdmissions,
) -> std::sync::MutexGuard<'_, Admissions> {
    admissions.lock().unwrap_or_else(PoisonError::into_inner)
}

impl NodeHandler {
    fn report(&self, event: ChannelEvent) {
        match self.commands.try_send(Command::Channel(event)) {
            Ok(()) | Err(TrySendError::Closed(_)) => {}
            Err(TrySendError::Full(command)) => {
                let commands = self.commands.clone();
                tokio::spawn(async move {
                    let _stopped = commands.send(command).await;
                });
            }
        }
    }
}

impl ChannelHandler for NodeHandler {
    fn networks(&self) -> Vec<NetworkId> {
        self.ledgers
            .read()
            .map(|ledgers| ledgers.keys().copied().collect())
            .unwrap_or_default()
    }

    fn with_ledger<R>(&self, network: NetworkId, f: impl FnOnce(&mut Ledger) -> R) -> Option<R> {
        let shared = self.ledgers.read().ok()?.get(&network).cloned()?;
        let mut ledger = shared.lock().ok()?;
        Some(f(&mut ledger))
    }

    fn handle_join(&self, network: NetworkId, proof: JoinProof) -> Result<(), String> {
        let result = self
            .with_ledger(network, |ledger| {
                sponsor_join(ledger, &self.identity, proof, now())
            })
            .ok_or_else(|| String::from("unknown network"))?;
        result.map_err(|error| error.to_string())?;
        self.report(ChannelEvent::LedgerChanged { network });
        Ok(())
    }

    fn on_endpoints(
        &self,
        network: NetworkId,
        from: NodeId,
        entries: Vec<(NodeId, Vec<SocketAddr>)>,
    ) {
        self.report(ChannelEvent::Endpoints {
            network,
            from,
            entries,
        });
    }

    fn on_presence(&self, network: NetworkId, from: NodeId, entries: Vec<(NodeId, Presence)>) {
        self.report(ChannelEvent::Presence {
            network,
            from,
            entries,
        });
    }

    fn on_ledger_changed(&self, network: NetworkId) {
        self.report(ChannelEvent::LedgerChanged { network });
    }

    fn handle_ask(&self, request: AdmissionRequest, from: SocketAddr) -> Result<(), String> {
        let network = request.network;
        let (device, name) = (request.device, request.name.clone());
        if self
            .with_ledger(network, |ledger| ledger.view().is_member(&device))
            .is_none()
        {
            return Err(String::from("unknown network"));
        }
        let new = lock_admissions(&self.admissions)
            .offer(request, Some(from), now())
            .map_err(|refusal| refusal.to_string())?;
        if new {
            self.report(ChannelEvent::Admissions {
                network,
                new: vec![(device, name)],
            });
        }
        Ok(())
    }

    fn on_admissions(
        &self,
        network: NetworkId,
        from: NodeId,
        requests: Vec<AdmissionRequest>,
        denied: Vec<(NodeId, Timestamp)>,
    ) {
        let Some((members, from_admin)) = self.with_ledger(network, |ledger| {
            let view = ledger.view();
            let members: BTreeSet<NodeId> = requests
                .iter()
                .map(|request| request.device)
                .filter(|device| view.is_member(device))
                .collect();
            (members, view.admins().contains(&from))
        }) else {
            return;
        };
        let at = now();
        let mut admissions = lock_admissions(&self.admissions);
        let mut changed = from_admin && admissions.absorb_denials(network, denied, at);
        let mut new = Vec::new();
        for request in requests {
            if request.network != network || members.contains(&request.device) {
                continue;
            }
            let (device, name) = (request.device, request.name.clone());
            if admissions.offer(request, None, at) == Ok(true) {
                new.push((device, name));
                changed = true;
            }
        }
        drop(admissions);
        if changed {
            self.report(ChannelEvent::Admissions { network, new });
        }
    }
}

#[derive(Debug)]
struct Task(JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug)]
struct Session {
    addrs: Vec<SocketAddr>,
    _task: Task,
}

/// The control channel: listener, outgoing sessions and the handler.
#[derive(Debug)]
pub(crate) struct ChannelLayer {
    channel: Channel<NodeHandler>,
    server: ChannelServer,
    local: NodeId,
    sessions: BTreeMap<NodeId, Session>,
    admissions: SharedAdmissions,
    _resync: Task,
}

impl ChannelLayer {
    /// Binds the control listener on `listen` and serves members and invite holders.
    pub(crate) async fn start(
        identity: Arc<DeviceIdentity>,
        ledgers: LedgerRegistry,
        listen: SocketAddr,
        commands: mpsc::Sender<Command>,
    ) -> Result<Self, NodeError> {
        let local = identity.node_id();
        let admissions = SharedAdmissions::default();
        let handler = Arc::new(NodeHandler {
            ledgers,
            identity: Arc::clone(&identity),
            commands,
            admissions: Arc::clone(&admissions),
        });
        let channel = Channel::new(identity.signing_key(), Arc::clone(&handler))?;
        let server = ChannelServer::bind(&channel, listen).await?;
        debug!(addr = %server.local_addr(), "control channel listening");
        let resync = Task(tokio::spawn(resync_loop(channel.clone(), handler)));
        Ok(Self {
            channel,
            server,
            local,
            sessions: BTreeMap::new(),
            admissions,
            _resync: resync,
        })
    }

    /// The bound listener address.
    pub(crate) const fn local_addr(&self) -> SocketAddr {
        self.server.local_addr()
    }

    /// Replaces the peers to keep sessions with, redialing closed ones with backoff.
    pub(crate) fn set_targets(&mut self, targets: Vec<SyncTarget>) -> Result<(), NodeError> {
        let mut wanted = BTreeMap::new();
        for target in targets {
            if target.node == self.local {
                return Err(NodeError::InvalidArgument(String::from(
                    "a sync target is this node",
                )));
            }
            if wanted.insert(target.node, target.addrs).is_some() {
                return Err(NodeError::InvalidArgument(format!(
                    "duplicate sync target {}",
                    target.node
                )));
            }
        }
        self.sessions.retain(|node, session| {
            wanted
                .get(node)
                .is_some_and(|addrs| *addrs == session.addrs)
        });
        for (node, addrs) in wanted {
            if self.sessions.contains_key(&node) || addrs.is_empty() {
                continue;
            }
            let task = Task(tokio::spawn(keep_session(
                self.channel.clone(),
                node,
                addrs.clone(),
            )));
            self.sessions.insert(node, Session { addrs, _task: task });
        }
        Ok(())
    }

    /// Re-syncs `network` with every session after a local ledger change.
    pub(crate) fn notify_changed(&self, network: NetworkId) {
        self.channel.notify_changed(network);
    }

    /// Gossips which of this gateway's plain WireGuard members of `network` are online.
    pub(crate) fn announce_presence(
        &self,
        network: NetworkId,
        entries: Vec<(NodeId, Presence)>,
    ) -> Result<(), NodeError> {
        Ok(self.channel.send_presence(network, entries)?)
    }

    /// Gossips endpoints for `network` to every connected member.
    pub(crate) fn gossip(
        &self,
        network: NetworkId,
        entries: Vec<(NodeId, Vec<SocketAddr>)>,
    ) -> Result<(), NodeError> {
        Ok(self.channel.send_endpoints(network, entries)?)
    }

    /// Join requests waiting for an admin, as the handler holds them.
    pub(crate) fn admissions(&self) -> &SharedAdmissions {
        &self.admissions
    }

    /// Gossips the join requests pending in `network` and the denied ones.
    pub(crate) fn gossip_admissions(
        &self,
        network: NetworkId,
        requests: Vec<AdmissionRequest>,
        denied: Vec<(NodeId, Timestamp)>,
    ) -> Result<(), NodeError> {
        Ok(self.channel.send_admissions(network, requests, denied)?)
    }

    /// Asks each bootstrap member in turn to hold `request` for an admin. `None` means it
    /// is pending; a ledger means an admin added this device.
    pub(crate) async fn ask(
        &self,
        bootstrap: &[Bootstrap],
        request: &AdmissionRequest,
    ) -> Result<Option<Ledger>, NodeError> {
        let mut last = None;
        for entry in bootstrap {
            for addr in bootstrap_addrs(entry).await {
                match ChannelClient::ask(&self.channel, addr, entry.node(), request.clone()).await {
                    Ok(answer) => return Ok(answer),
                    Err(error @ ChannelError::JoinRejected(_)) => return Err(error.into()),
                    Err(error) => {
                        debug!(%addr, node = %entry.node(), %error, "bootstrap entry failed");
                        last = Some(error.into());
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| {
            NodeError::InvalidArgument(String::from("no member in the code could be reached"))
        }))
    }

    /// Tries each bootstrap entry, pinned to that entry's node, and returns the ledger.
    pub(crate) async fn join(
        &self,
        token: &InviteToken,
        proof: JoinProof,
    ) -> Result<Ledger, NodeError> {
        let mut last = None;
        for entry in token.bootstrap() {
            let addrs: Vec<SocketAddr> = match entry.addr() {
                BootstrapAddr::Socket(addr) => vec![*addr],
                BootstrapAddr::Host(host, port) => {
                    match tokio::net::lookup_host((host.as_str(), *port)).await {
                        Ok(found) => found.collect(),
                        Err(error) => {
                            debug!(%entry, %error, "bootstrap host did not resolve");
                            last = Some(NodeError::Io(error));
                            continue;
                        }
                    }
                }
            };
            for addr in addrs {
                match ChannelClient::join(
                    &self.channel,
                    addr,
                    entry.node(),
                    token.network(),
                    proof.clone(),
                )
                .await
                {
                    Ok(ledger) => return Ok(ledger),
                    Err(error @ ChannelError::JoinRejected(_)) => return Err(error.into()),
                    Err(error) => {
                        debug!(%addr, node = %entry.node(), %error, "bootstrap entry failed");
                        last = Some(error.into());
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| {
            NodeError::InvalidArgument(String::from("the invite has no bootstrap entries"))
        }))
    }
}

async fn bootstrap_addrs(entry: &Bootstrap) -> Vec<SocketAddr> {
    match entry.addr() {
        BootstrapAddr::Socket(addr) => vec![*addr],
        BootstrapAddr::Host(host, port) => {
            match tokio::net::lookup_host((host.as_str(), *port)).await {
                Ok(found) => found.collect(),
                Err(error) => {
                    debug!(%entry, %error, "bootstrap host did not resolve");
                    Vec::new()
                }
            }
        }
    }
}

async fn resync_loop(channel: Channel<NodeHandler>, handler: Arc<NodeHandler>) {
    let mut ticks = tokio::time::interval(RESYNC_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticks.tick().await;
    loop {
        ticks.tick().await;
        for network in handler.networks() {
            channel.notify_changed(network);
        }
    }
}

async fn keep_session(channel: Channel<NodeHandler>, node: NodeId, addrs: Vec<SocketAddr>) {
    let mut backoff = BACKOFF_MIN;
    loop {
        for addr in &addrs {
            match ChannelClient::connect(&channel, *addr, node).await {
                Ok(mut client) => {
                    debug!(%node, %addr, "control session up");
                    backoff = BACKOFF_MIN;
                    client.closed().await;
                    debug!(%node, %addr, "control session closed");
                    break;
                }
                Err(error) => debug!(%node, %addr, %error, "control dial failed"),
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = backoff.saturating_mul(2).min(BACKOFF_MAX);
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the node driver reads the wall clock to check invite expiry"
)]
pub(crate) fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    Timestamp::from_millis(millis)
}

/// Chooses sync targets: every relay, every gateway of a plain WireGuard member (only it
/// can say whether that member is online), then up to [`MAX_DIRECT_SESSIONS`] reachable peers.
pub(crate) fn pick_targets(
    netmaps: &[&NetMap],
    relays: &[Relay],
    endpoints: &BTreeMap<NodeId, Vec<SocketAddr>>,
) -> Vec<SyncTarget> {
    let mut targets = Vec::new();
    let mut chosen = BTreeSet::new();
    let members: BTreeSet<NodeId> = netmaps
        .iter()
        .filter(|netmap| netmap.is_member())
        .flat_map(|netmap| netmap.peers().keys().copied())
        .collect();
    for relay in relays {
        let node = relay.node();
        if !members.contains(&node) || !chosen.insert(node) {
            continue;
        }
        let mut addrs: Vec<SocketAddr> = relay.address().parse().into_iter().collect();
        for addr in endpoints.get(&node).into_iter().flatten() {
            if !addrs.contains(addr) {
                addrs.push(*addr);
            }
        }
        if addrs.is_empty() {
            debug!(relay = relay.name(), "relay has no dialable address yet");
            chosen.remove(&node);
            continue;
        }
        targets.push(SyncTarget { node, addrs });
    }
    let gateways: BTreeSet<NodeId> = netmaps
        .iter()
        .flat_map(|netmap| netmap.clients().values().map(|client| client.gateway))
        .filter(|gateway| members.contains(gateway))
        .collect();
    for gateway in gateways {
        let Some(addrs) = endpoints.get(&gateway).filter(|addrs| !addrs.is_empty()) else {
            continue;
        };
        if chosen.insert(gateway) {
            targets.push(SyncTarget {
                node: gateway,
                addrs: addrs.clone(),
            });
        }
    }
    let direct = members
        .iter()
        .filter(|node| !chosen.contains(*node))
        .filter_map(|node| {
            let addrs = endpoints.get(node).filter(|addrs| !addrs.is_empty())?;
            Some(SyncTarget {
                node: *node,
                addrs: addrs.clone(),
            })
        })
        .take(MAX_DIRECT_SESSIONS);
    targets.extend(direct);
    targets
}
