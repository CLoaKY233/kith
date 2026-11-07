//! The network state derived from the valid ops of a ledger.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

use kith_crypto::VerifyingKey;
use kith_types::{AccountId, NetworkId, NodeId, OpId, Timestamp};

use crate::{Credential, NetworkParams, Role};

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
    /// For a plain WireGuard device, how it is reached; `None` for a kith device.
    pub via: Option<Via>,
}

/// How a plain WireGuard device, such as a phone, is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Via {
    /// The kith device it connects to.
    pub gateway: NodeId,
    /// Its WireGuard public key.
    pub key: [u8; 32],
    /// Its IPv4 address, fixed when it was added.
    pub v4: Ipv4Addr,
    /// Its IPv6 address, fixed when it was added.
    pub v6: Ipv6Addr,
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
    /// The author's wall-clock time when the invite was created.
    pub created: Timestamp,
}

/// A person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Their name.
    pub name: String,
    /// What they may do.
    pub role: Role,
    /// Their passkeys; the first one's key names the account.
    pub credentials: Vec<Credential>,
    /// The op that added them.
    pub added_by: OpId,
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
    pub(crate) accounts: BTreeMap<AccountId, Account>,
    pub(crate) consoles: BTreeSet<NodeId>,
    pub(crate) ever_consoles: BTreeSet<NodeId>,
    pub(crate) recovery_key: Option<VerifyingKey>,
    pub(crate) device_owners: BTreeMap<NodeId, AccountId>,
    pub(crate) account_changes: Vec<OpId>,
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

    /// The people; empty in a network that never added one.
    pub const fn accounts(&self) -> &BTreeMap<AccountId, Account> {
        &self.accounts
    }

    /// One person.
    pub fn account(&self, account: &AccountId) -> Option<&Account> {
        self.accounts.get(account)
    }

    /// The devices authorized to sign admin ops on behalf of people.
    pub const fn consoles(&self) -> &BTreeSet<NodeId> {
        &self.consoles
    }

    /// Whether `device` is an authorized console.
    pub fn is_console(&self, device: &NodeId) -> bool {
        self.consoles.contains(device)
    }

    /// Whether `device` was ever an authorized console. Such a device signs admin ops only
    /// on behalf of a person, even after it is revoked.
    pub fn was_console(&self, device: &NodeId) -> bool {
        self.ever_consoles.contains(device)
    }

    /// The recovery key's public half, once set.
    pub const fn recovery_key(&self) -> Option<&VerifyingKey> {
        self.recovery_key.as_ref()
    }

    /// Whose each member device is, for those assigned to a current account.
    pub const fn device_owners(&self) -> &BTreeMap<NodeId, AccountId> {
        &self.device_owners
    }

    /// The member devices assigned to `account`.
    pub fn devices_of<'a>(&'a self, account: &'a AccountId) -> impl Iterator<Item = NodeId> + 'a {
        self.device_owners
            .iter()
            .filter(move |(_, owner)| *owner == account)
            .map(|(device, _)| *device)
    }

    /// The changes to people, consoles and the recovery key that took effect, oldest
    /// first: they form one chain, so an op missing here was refused or lost a race.
    pub fn account_changes(&self) -> &[OpId] {
        &self.account_changes
    }
}
