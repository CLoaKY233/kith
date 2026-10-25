#![expect(
    clippy::needless_pass_by_value,
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    clippy::unused_self,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use core::net::SocketAddr;
use std::collections::BTreeMap;
use std::sync::Arc;

use kith_control::NetMap;
use kith_control::records::Relay;
use kith_identity::{DeviceIdentity, InviteToken};
use kith_ledger::{JoinProof, Ledger};
use kith_types::{NetworkId, NodeId};
use tokio::sync::mpsc;

use crate::network::LedgerRegistry;
use crate::types::{Command, NodeError};

/// Most direct peers a node keeps a control connection to, besides relays.
pub(crate) const MAX_DIRECT_SESSIONS: usize = 8;

/// How often every session re-syncs as a safety net.
pub(crate) const RESYNC_INTERVAL: core::time::Duration = core::time::Duration::from_secs(60);

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

/// The control channel: listener, outgoing sessions and the handler.
#[derive(Debug)]
pub(crate) struct ChannelLayer {
    handler: Arc<NodeHandler>,
    local_addr: SocketAddr,
}

impl ChannelLayer {
    /// Binds the control listener on `listen` and serves members and invite holders.
    pub(crate) async fn start(
        identity: Arc<DeviceIdentity>,
        ledgers: LedgerRegistry,
        listen: SocketAddr,
        commands: mpsc::Sender<Command>,
    ) -> Result<Self, NodeError> {
        Err(NodeError::Pending("channel: start"))
    }

    /// The bound listener address.
    pub(crate) const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Replaces the peers to keep sessions with, redialing closed ones with backoff.
    pub(crate) fn set_targets(&mut self, targets: Vec<SyncTarget>) -> Result<(), NodeError> {
        Err(NodeError::Pending("channel: set targets"))
    }

    /// Re-syncs `network` with every session after a local ledger change.
    pub(crate) fn notify_changed(&self, network: NetworkId) {}

    /// Gossips endpoints for `network` to every connected member.
    pub(crate) fn gossip(
        &self,
        network: NetworkId,
        entries: Vec<(NodeId, Vec<SocketAddr>)>,
    ) -> Result<(), NodeError> {
        Err(NodeError::Pending("channel: gossip"))
    }

    /// Tries the token's bootstrap addresses, pinned to its sponsor, and returns the ledger.
    pub(crate) async fn join(
        &self,
        token: &InviteToken,
        proof: JoinProof,
    ) -> Result<Ledger, NodeError> {
        Err(NodeError::Pending("channel: join"))
    }
}

/// Chooses sync targets: every relay, then up to [`MAX_DIRECT_SESSIONS`] reachable peers.
pub(crate) fn pick_targets(
    netmaps: &[&NetMap],
    relays: &[Relay],
    endpoints: &BTreeMap<NodeId, Vec<SocketAddr>>,
) -> Vec<SyncTarget> {
    Vec::new()
}
