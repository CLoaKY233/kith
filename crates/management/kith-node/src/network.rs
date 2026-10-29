use core::net::{Ipv4Addr, Ipv6Addr};
use core::time::Duration;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use kith_control::{NetMap, NetMapDiff, Networks, build_netmap, records};
use kith_identity::{Bootstrap, DeviceIdentity, InviteToken};
use kith_ledger::{JoinProof, Ledger, NetworkParams, OpBody, SignedOp};
use kith_policy::{Addresses, Policy};
use kith_types::{Ipv4Prefix, Ipv6Prefix, NetworkId, NodeId, Timestamp};

use crate::types::NodeError;

/// A ledger shared between the loop and control connections; never locked across `.await`.
pub(crate) type SharedLedger = Arc<Mutex<Ledger>>;

/// Every joined network's ledger, as the control channel's handler sees them.
pub(crate) type LedgerRegistry = Arc<RwLock<BTreeMap<NetworkId, SharedLedger>>>;

/// One joined network: its shared ledger and the network map last derived from it.
#[derive(Debug)]
pub(crate) struct NetworkRuntime {
    id: NetworkId,
    local: NodeId,
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
        let params = NetworkParams {
            ipv4: Ipv4Prefix::new(Ipv4Addr::new(100, 64, 0, 0), 10)
                .map_err(|e| NodeError::InvalidArgument(e.to_string()))?,
            ipv6: Ipv6Prefix::new(Ipv6Addr::new(0xfd7a, 0x115c, 0xa1e0, 0, 0, 0, 0, 0), 48)
                .map_err(|e| NodeError::InvalidArgument(e.to_string()))?,
        };
        let ledger = Ledger::create(
            identity.signing_key(),
            name,
            device_name,
            params,
            &[],
            1,
            now,
        )?;
        Self::from_ledger(ledger, identity.node_id())
    }

    /// Wraps a ledger received by joining, or rebuilt from storage.
    pub(crate) fn from_ledger(ledger: Ledger, local: NodeId) -> Result<Self, NodeError> {
        let netmap = build_netmap(ledger.view(), local)?;
        Ok(Self {
            id: ledger.network_id(),
            local,
            ledger: Arc::new(Mutex::new(ledger)),
            netmap,
        })
    }

    /// Rebuilds a ledger from persisted ops; the genesis op may be anywhere in the list.
    pub(crate) fn from_ops(ops: Vec<SignedOp>, local: NodeId) -> Result<Self, NodeError> {
        let genesis = ops
            .iter()
            .position(|op| matches!(op.op().body, OpBody::Genesis { .. }))
            .ok_or_else(|| NodeError::InvalidArgument(String::from("no genesis op")))?;
        let mut ops = ops;
        let mut ledger = Ledger::new(ops.swap_remove(genesis))?;
        if let Some(error) = ledger.insert_batch(ops).first_error() {
            return Err(error.clone().into());
        }
        Self::from_ledger(ledger, local)
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

    /// Re-derives the map from the ledger; `None` when it did not change.
    pub(crate) fn rebuild(&mut self, local: NodeId) -> Result<Option<Change>, NodeError> {
        let netmap = self.with_ledger(|ledger| build_netmap(ledger.view(), local))??;
        self.local = local;
        let diff = NetMap::diff(&self.netmap, &netmap);
        self.netmap = netmap;
        Ok((!diff.is_empty()).then_some(Change {
            network: self.id,
            diff,
        }))
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
        let expires = now
            .checked_add(expires_in)
            .ok_or_else(|| NodeError::InvalidArgument(String::from("invite expiry overflows")))?;
        let author = identity.node_id();
        self.with_ledger(|ledger| {
            let token = InviteToken::generate(
                ledger.network_id(),
                author,
                bootstrap,
                expires,
                String::from(ledger.view().name()),
            )?;
            let body = OpBody::CreateInvite {
                invite_key: token.invite_public_key(),
                expires,
                max_uses,
                tags: Vec::new(),
            };
            let op = ledger
                .draft(author, body, now)
                .sign(identity.signing_key())?;
            ledger.insert(op)?;
            Ok(token)
        })?
    }

    /// Signs and inserts a `RemoveDevice` op.
    pub(crate) fn remove_device(
        &mut self,
        identity: &DeviceIdentity,
        device: NodeId,
        now: Timestamp,
    ) -> Result<(), NodeError> {
        let author = identity.node_id();
        self.with_ledger(|ledger| {
            let op = ledger
                .draft(author, OpBody::RemoveDevice { device }, now)
                .sign(identity.signing_key())?;
            ledger.insert(op)?;
            Ok(())
        })?
    }

    /// The addresses a plain WireGuard device with public `key` would be given now.
    pub(crate) fn client_addresses(&self, key: &[u8; 32]) -> Result<Addresses, NodeError> {
        self.with_ledger(|ledger| {
            Ok(kith_control::addresses_for_newcomer(
                ledger.view(),
                kith_ledger::client_id(key),
            )?)
        })?
    }

    /// Signs and inserts an `AddClient` op admitting a plain WireGuard device with public
    /// `key` through `identity`'s device, at `addresses` (from [`client_addresses`](Self::client_addresses)).
    pub(crate) fn add_client(
        &mut self,
        identity: &DeviceIdentity,
        key: [u8; 32],
        (name, tags): (String, Vec<String>),
        addresses: Addresses,
        now: Timestamp,
    ) -> Result<(), NodeError> {
        let author = identity.node_id();
        self.with_ledger(|ledger| {
            let body = OpBody::AddClient {
                key,
                gateway: author,
                name,
                tags,
                v4: addresses.v4,
                v6: addresses.v6,
            };
            let op = ledger
                .draft(author, body, now)
                .sign(identity.signing_key())?;
            ledger.insert(op)?;
            Ok(())
        })?
    }

    /// Signs and inserts a `SetRecord` op; an empty value deletes the record.
    pub(crate) fn set_record(
        &mut self,
        identity: &DeviceIdentity,
        key: String,
        value: Vec<u8>,
        now: Timestamp,
    ) -> Result<(), NodeError> {
        let value = (!value.is_empty()).then_some(value);
        let author = identity.node_id();
        self.with_ledger(|ledger| {
            let op = ledger
                .draft(author, OpBody::SetRecord { key, value }, now)
                .sign(identity.signing_key())?;
            ledger.insert(op)?;
            Ok(())
        })?
    }

    /// Signs and inserts the policy op.
    pub(crate) fn set_policy(
        &mut self,
        identity: &DeviceIdentity,
        policy: &Policy,
        now: Timestamp,
    ) -> Result<(), NodeError> {
        let bytes = policy
            .to_bytes()
            .map_err(|e| NodeError::InvalidArgument(e.to_string()))?;
        self.set_record(identity, String::from(records::POLICY), bytes, now)
    }
}

/// Checks a join proof against our clock, then drafts, signs and inserts the `Join` op.
pub(crate) fn sponsor_join(
    ledger: &mut Ledger,
    identity: &DeviceIdentity,
    proof: JoinProof,
    now: Timestamp,
) -> Result<(), NodeError> {
    let invite = ledger
        .view()
        .invites()
        .get(&proof.invite_key.to_bytes())
        .ok_or_else(|| NodeError::InvalidArgument(String::from("unknown or used-up invite")))?;
    if now > invite.expires {
        return Err(NodeError::InvalidArgument(String::from("invite expired")));
    }
    let device = proof.device;
    let author = identity.node_id();
    let op = ledger
        .draft(author, OpBody::Join(proof), now)
        .sign(identity.signing_key())?;
    ledger.insert(op)?;
    if ledger.view().is_member(&device) {
        Ok(())
    } else {
        Err(NodeError::InvalidArgument(String::from(
            "join was not admitted",
        )))
    }
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
        let id = runtime.id;
        let local = runtime.local;
        let ledger = runtime.ledger();
        let diff = {
            let locked = ledger.lock().map_err(|_| NodeError::Poisoned)?;
            self.maps.update(locked.view(), local)
        };
        diff?;
        self.registry
            .write()
            .map_err(|_| NodeError::Poisoned)?
            .insert(id, ledger);
        self.runtimes.insert(id, runtime);
        Ok(())
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
        let runtime = self
            .runtimes
            .get_mut(network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?;
        let change = runtime.rebuild(local)?;
        if change.is_some() {
            let ledger = runtime.ledger();
            let locked = ledger.lock().map_err(|_| NodeError::Poisoned)?;
            self.maps.update(locked.view(), local)?;
        }
        Ok(change)
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
