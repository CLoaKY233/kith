#![expect(
    clippy::needless_pass_by_value,
    clippy::unused_self,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use core::time::Duration;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use kith_control::{NetMap, NetMapDiff, Networks};
use kith_identity::{Bootstrap, DeviceIdentity, InviteToken};
use kith_ledger::{JoinProof, Ledger, SignedOp};
use kith_policy::Policy;
use kith_types::{NetworkId, NodeId, Timestamp};

use crate::types::NodeError;

/// A ledger shared between the loop and control connections; never locked across `.await`.
pub(crate) type SharedLedger = Arc<Mutex<Ledger>>;

/// Every joined network's ledger, as the control channel's handler sees them.
pub(crate) type LedgerRegistry = Arc<RwLock<BTreeMap<NetworkId, SharedLedger>>>;

/// One joined network: its shared ledger and the network map last derived from it.
#[derive(Debug)]
pub(crate) struct NetworkRuntime {
    id: NetworkId,
    ledger: SharedLedger,
    netmap: NetMap,
}

/// What changed when a runtime's ledger moved.
#[derive(Debug)]
pub(crate) struct Change {
    /// The network.
    pub(crate) network: NetworkId,
    /// How its network map moved.
    pub(crate) diff: NetMapDiff,
}

impl NetworkRuntime {
    /// Founds a network with `identity` as its only admin and member.
    pub(crate) fn create(
        identity: &DeviceIdentity,
        name: &str,
        device_name: &str,
        now: Timestamp,
    ) -> Result<Self, NodeError> {
        Err(NodeError::Pending("network: create"))
    }

    /// Wraps a ledger received by joining, or rebuilt from storage.
    pub(crate) fn from_ledger(ledger: Ledger, local: NodeId) -> Result<Self, NodeError> {
        Err(NodeError::Pending("network: from ledger"))
    }

    /// Rebuilds a ledger from persisted ops; the genesis op may be anywhere in the list.
    pub(crate) fn from_ops(ops: Vec<SignedOp>, local: NodeId) -> Result<Self, NodeError> {
        Err(NodeError::Pending("network: from ops"))
    }

    /// The network's id.
    pub(crate) const fn id(&self) -> NetworkId {
        self.id
    }

    /// The shared ledger, for the control channel.
    pub(crate) fn ledger(&self) -> SharedLedger {
        Arc::clone(&self.ledger)
    }

    /// The network map as of the last rebuild.
    pub(crate) const fn netmap(&self) -> &NetMap {
        &self.netmap
    }

    /// Runs `f` with the ledger locked.
    pub(crate) fn with_ledger<R>(&self, f: impl FnOnce(&mut Ledger) -> R) -> Result<R, NodeError> {
        let mut ledger = self.ledger.lock().map_err(|_| NodeError::Poisoned)?;
        Ok(f(&mut ledger))
    }

    /// Every op of the ledger, for persistence.
    pub(crate) fn ops(&self) -> Result<Vec<SignedOp>, NodeError> {
        self.with_ledger(|ledger| {
            ledger
                .op_ids()
                .filter_map(|id| ledger.get(id))
                .cloned()
                .collect()
        })
    }

    /// Re-derives the map from the ledger; `None` when it did not change.
    pub(crate) fn rebuild(&mut self, local: NodeId) -> Result<Option<Change>, NodeError> {
        Err(NodeError::Pending("network: rebuild"))
    }

    /// Signs and inserts a `CreateInvite` op and returns its token.
    pub(crate) fn invite(
        &mut self,
        identity: &DeviceIdentity,
        expires_in: Duration,
        max_uses: u16,
        bootstrap: Vec<Bootstrap>,
        now: Timestamp,
    ) -> Result<InviteToken, NodeError> {
        Err(NodeError::Pending("network: invite"))
    }

    /// Signs and inserts a `SetRecord` op; an empty value deletes the record.
    pub(crate) fn set_record(
        &mut self,
        identity: &DeviceIdentity,
        key: String,
        value: Vec<u8>,
        now: Timestamp,
    ) -> Result<(), NodeError> {
        Err(NodeError::Pending("network: set record"))
    }

    /// Signs and inserts the policy op.
    pub(crate) fn set_policy(
        &mut self,
        identity: &DeviceIdentity,
        policy: &Policy,
        now: Timestamp,
    ) -> Result<(), NodeError> {
        Err(NodeError::Pending("network: set policy"))
    }
}

/// Checks a join proof against our clock, then drafts, signs and inserts the `Join` op.
pub(crate) fn sponsor_join(
    ledger: &mut Ledger,
    identity: &DeviceIdentity,
    proof: JoinProof,
    now: Timestamp,
) -> Result<(), NodeError> {
    Err(NodeError::Pending("network: sponsor join"))
}

/// Every joined network, the cross-network map set, and the registry the channel reads.
#[derive(Debug, Default)]
pub(crate) struct NetworkSet {
    runtimes: BTreeMap<NetworkId, NetworkRuntime>,
    maps: Networks,
    registry: LedgerRegistry,
}

impl NetworkSet {
    /// Adds or replaces a runtime and publishes its ledger to the registry.
    pub(crate) fn insert(&mut self, runtime: NetworkRuntime) -> Result<(), NodeError> {
        Err(NodeError::Pending("network: set insert"))
    }

    /// Removes a runtime and withdraws its ledger from the registry.
    pub(crate) fn remove(
        &mut self,
        network: &NetworkId,
    ) -> Result<Option<NetworkRuntime>, NodeError> {
        self.registry
            .write()
            .map_err(|_| NodeError::Poisoned)?
            .remove(network);
        self.maps.remove(network);
        Ok(self.runtimes.remove(network))
    }

    /// A runtime by id.
    pub(crate) fn get(&self, network: &NetworkId) -> Option<&NetworkRuntime> {
        self.runtimes.get(network)
    }

    /// Rebuilds one runtime's map and copies it into the map set.
    pub(crate) fn rebuild(
        &mut self,
        network: &NetworkId,
        local: NodeId,
    ) -> Result<Option<Change>, NodeError> {
        Err(NodeError::Pending("network: set rebuild"))
    }

    /// A runtime by id, mutably; call [`NetworkSet::rebuild`] after changing its ledger.
    pub(crate) fn get_mut(&mut self, network: &NetworkId) -> Option<&mut NetworkRuntime> {
        self.runtimes.get_mut(network)
    }

    /// Every runtime.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &NetworkRuntime> {
        self.runtimes.values()
    }

    /// Every network map.
    pub(crate) const fn maps(&self) -> &Networks {
        &self.maps
    }

    /// The ledger registry, shared with the control channel's handler.
    pub(crate) fn registry(&self) -> LedgerRegistry {
        Arc::clone(&self.registry)
    }
}
