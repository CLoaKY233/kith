//! The network state derived from the valid ops of a ledger.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use kith_crypto::VerifyingKey;
use kith_types::{NetworkId, NodeId, OpId, Timestamp};

use crate::NetworkParams;

/// A member device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The device's name.
    pub name: String,
    /// The device's tags, sorted and unique.
    pub tags: Vec<String>,
    /// The op that admitted it: genesis, an `AddDevice` or a `Join`. After a re-add, the
    /// latest admitting op.
    pub admitted_by: OpId,
    /// The lamport clock of `admitted_by`.
    pub admitted_at_lamport: u64,
}

/// An invite that can still admit devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// The invite's public key.
    pub key: VerifyingKey,
    /// The last instant a join may claim. The ledger has no clock, so an invite listed
    /// here may already have expired; compare with the current time.
    pub expires: Timestamp,
    /// How many joins it admits in total.
    pub max_uses: u16,
    /// How many valid joins have used it. Always below `max_uses`.
    pub uses: u16,
    /// Tags given to joining devices.
    pub tags: Vec<String>,
}

/// The state of a network, derived deterministically from the set of ops alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkView {
    pub(crate) network: NetworkId,
    pub(crate) name: String,
    pub(crate) params: NetworkParams,
    pub(crate) admins: BTreeSet<NodeId>,
    pub(crate) threshold: u8,
    pub(crate) devices: BTreeMap<NodeId, Device>,
    pub(crate) invites: BTreeMap<[u8; 32], Invite>,
    pub(crate) records: BTreeMap<String, Vec<u8>>,
    pub(crate) heads: Vec<OpId>,
}

impl NetworkView {
    /// The network's identity.
    pub const fn network_id(&self) -> NetworkId {
        self.network
    }

    /// The network's name, from genesis.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The network's settings, from genesis.
    pub const fn params(&self) -> &NetworkParams {
        &self.params
    }

    /// The current admins.
    pub const fn admins(&self) -> &BTreeSet<NodeId> {
        &self.admins
    }

    /// Admin signatures that authority ops currently need.
    pub const fn threshold(&self) -> u8 {
        self.threshold
    }

    /// The member devices.
    pub const fn devices(&self) -> &BTreeMap<NodeId, Device> {
        &self.devices
    }

    /// Whether `device` is a member.
    pub fn is_member(&self, device: &NodeId) -> bool {
        self.devices.contains_key(device)
    }

    /// Invites that are neither revoked nor used up, keyed by public key bytes.
    pub const fn invites(&self) -> &BTreeMap<[u8; 32], Invite> {
        &self.invites
    }

    /// The key-value records.
    pub const fn records(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.records
    }

    /// The DAG tips, sorted.
    pub fn heads(&self) -> &[OpId] {
        &self.heads
    }
}
