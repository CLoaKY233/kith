use core::net::SocketAddr;
use core::time::Duration;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use kith_channel::{Channel, ChannelClient, ChannelError, ChannelHandler, ChannelServer};
use kith_control::NetMap;
use kith_control::records::Relay;
use kith_identity::{BootstrapAddr, DeviceIdentity, InviteToken};
use kith_ledger::{JoinProof, Ledger};
use kith_types::{NetworkId, NodeId, Timestamp};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::task::JoinHandle;
use tracing::debug;

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

    fn on_ledger_changed(&self, network: NetworkId) {
        self.report(ChannelEvent::LedgerChanged { network });
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
        let handler = Arc::new(NodeHandler {
            ledgers,
            identity: Arc::clone(&identity),
            commands,
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

    /// Gossips endpoints for `network` to every connected member.
    pub(crate) fn gossip(
        &self,
        network: NetworkId,
        entries: Vec<(NodeId, Vec<SocketAddr>)>,
    ) -> Result<(), NodeError> {
        Ok(self.channel.send_endpoints(network, entries)?)
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

/// Chooses sync targets: every relay, then up to [`MAX_DIRECT_SESSIONS`] reachable peers.
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
