//! Deriving a [`NetMap`] from a network view, and diffing two maps.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::net::IpAddr;
use core::ops::Bound;

use kith_ledger::{Device, NetworkView};
use kith_policy::{
    Addresses, AllocationError, CompiledFilter, Member, Members, Policy, TagName, allocate,
    compile, resolver_address,
};
use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NetworkId, NodeId};
use kith_wire::WireError;

use crate::records;

/// Why a network map could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ControlError {
    /// The network's mesh ranges cannot hold its members.
    #[error("address allocation failed: {0}")]
    Allocation(#[from] AllocationError),
}

/// Something wrong in the ledger's records that the map works around.
///
/// Warnings never stop a map from being built; the UI shows them to admins.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// The policy record does not decode. The map uses deny-all instead.
    InvalidPolicy(WireError),
    /// A device carries a tag that is not a valid policy tag. Policy rules ignore it.
    InvalidTag {
        /// The device.
        device: NodeId,
        /// The tag as stored in the ledger.
        tag: String,
    },
    /// A `route/…` record has a malformed key or value. It is ignored.
    InvalidRoute {
        /// The record key.
        key: String,
    },
    /// A route is advertised by a device that is not a member. It is ignored.
    RouteFromNonMember {
        /// The advertised prefix.
        prefix: IpPrefix,
        /// The named device.
        device: NodeId,
    },
}

/// A member device other than the local one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// The device's name.
    pub name: String,
    /// The device's valid tags.
    pub tags: BTreeSet<TagName>,
    /// The device's mesh addresses.
    pub addresses: Addresses,
}

/// Everything the other planes need to know about one network, from the local device's
/// point of view.
///
/// Built by [`build_netmap`]; a pure function of the view and the local device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetMap {
    network: NetworkId,
    name: String,
    is_member: bool,
    local_addresses: Option<Addresses>,
    resolver: [IpAddr; 2],
    ipv4_range: Ipv4Prefix,
    ipv6_range: Ipv6Prefix,
    peers: BTreeMap<NodeId, Peer>,
    routes: BTreeMap<IpPrefix, NodeId>,
    filter: CompiledFilter,
    warnings: Vec<Warning>,
}

impl NetMap {
    /// The network's identity.
    pub const fn network_id(&self) -> NetworkId {
        self.network
    }

    /// The network's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the local device is a member. When it is not (removed, or not yet joined),
    /// the map has no peers, no routes, no local addresses, and a filter that allows
    /// nothing.
    pub const fn is_member(&self) -> bool {
        self.is_member
    }

    /// The local device's mesh addresses, if it is a member.
    pub const fn local_addresses(&self) -> Option<Addresses> {
        self.local_addresses
    }

    /// The mesh DNS resolver's addresses: IPv4 first, then IPv6.
    pub const fn resolver(&self) -> [IpAddr; 2] {
        self.resolver
    }

    /// The IPv4 mesh range.
    pub const fn ipv4_range(&self) -> Ipv4Prefix {
        self.ipv4_range
    }

    /// The IPv6 mesh range.
    pub const fn ipv6_range(&self) -> Ipv6Prefix {
        self.ipv6_range
    }

    /// Every member except the local device.
    pub const fn peers(&self) -> &BTreeMap<NodeId, Peer> {
        &self.peers
    }

    /// Subnet routes advertised by members: prefix → advertising device. May include the
    /// local device's own advertisements.
    pub const fn routes(&self) -> &BTreeMap<IpPrefix, NodeId> {
        &self.routes
    }

    /// The local device's inbound packet filter.
    pub const fn filter(&self) -> &CompiledFilter {
        &self.filter
    }

    /// Problems found in the ledger's records, in a deterministic order.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// What changed from `old` to `new`.
    pub fn diff(old: &Self, new: &Self) -> NetMapDiff {
        let mut diff = NetMapDiff {
            filter_changed: old.filter != new.filter,
            local_addresses_changed: old.local_addresses != new.local_addresses,
            routes_changed: old.routes != new.routes,
            ..NetMapDiff::default()
        };
        for (node, peer) in &new.peers {
            match old.peers.get(node) {
                None => {
                    diff.peers_added.insert(*node, peer.clone());
                }
                Some(previous) if previous != peer => {
                    diff.peers_changed.insert(*node, peer.clone());
                }
                Some(_) => {}
            }
        }
        diff.peers_removed = old
            .peers
            .keys()
            .filter(|node| !new.peers.contains_key(node))
            .copied()
            .collect();
        diff
    }

    /// The diff from nothing to `self`: every peer added, everything else changed.
    pub(crate) fn initial_diff(&self) -> NetMapDiff {
        NetMapDiff {
            peers_added: self.peers.clone(),
            filter_changed: true,
            local_addresses_changed: true,
            routes_changed: true,
            ..NetMapDiff::default()
        }
    }
}

/// The minimal changes between two maps of the same network.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetMapDiff {
    /// Peers that are new, with their state.
    pub peers_added: BTreeMap<NodeId, Peer>,
    /// Peers that are gone.
    pub peers_removed: BTreeSet<NodeId>,
    /// Peers whose name, tags or addresses changed, with their new state.
    pub peers_changed: BTreeMap<NodeId, Peer>,
    /// Whether the inbound filter changed.
    pub filter_changed: bool,
    /// Whether the local device's addresses changed (including gaining or losing them).
    pub local_addresses_changed: bool,
    /// Whether the subnet routes changed.
    pub routes_changed: bool,
}

impl NetMapDiff {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.peers_added.is_empty()
            && self.peers_removed.is_empty()
            && self.peers_changed.is_empty()
            && !self.filter_changed
            && !self.local_addresses_changed
            && !self.routes_changed
    }

    /// Applies the peer changes to `peers`, turning the old map's peers into the new one's.
    pub fn apply_to_peers(&self, peers: &mut BTreeMap<NodeId, Peer>) {
        for node in &self.peers_removed {
            peers.remove(node);
        }
        for (node, peer) in self.peers_added.iter().chain(&self.peers_changed) {
            peers.insert(*node, peer.clone());
        }
    }
}

/// Builds the map of `view` as seen by `local`.
///
/// Fails only if the network's ranges cannot hold its members; everything wrong in records
/// becomes a [`Warning`]. A `local` that is not a member gets an empty map with
/// [`NetMap::is_member`] false, so the UI can say the device was removed.
pub fn build_netmap(view: &NetworkView, local: NodeId) -> Result<NetMap, ControlError> {
    let params = view.params();
    let resolver = [
        resolver_address(IpPrefix::V4(params.ipv4))?,
        resolver_address(IpPrefix::V6(params.ipv6))?,
    ];
    let mut warnings = Vec::new();
    let is_member = view.is_member(&local);

    let (members, peers, routes, filter) = if is_member {
        let addresses = allocate(params.ipv4, params.ipv6, &admission_order(view))?;
        let members: Members = view
            .devices()
            .iter()
            .filter_map(|(node, device)| {
                let addresses = *addresses.get(node)?;
                let tags = valid_tags(*node, device, &mut warnings);
                Some((*node, Member { tags, addresses }))
            })
            .collect();
        let peers = members
            .iter()
            .filter(|(node, _)| **node != local)
            .filter_map(|(node, member)| {
                let device = view.devices().get(node)?;
                let peer = Peer {
                    name: device.name.clone(),
                    tags: member.tags.clone(),
                    addresses: member.addresses,
                };
                Some((*node, peer))
            })
            .collect();
        let policy = policy(view, &mut warnings);
        let filter = compile(&policy, &members, local);
        let routes = routes(view, &mut warnings);
        (Some(members), peers, routes, filter)
    } else {
        (
            None,
            BTreeMap::new(),
            BTreeMap::new(),
            CompiledFilter::default(),
        )
    };

    Ok(NetMap {
        network: view.network_id(),
        name: String::from(view.name()),
        is_member,
        local_addresses: members
            .as_ref()
            .and_then(|members| members.get(&local))
            .map(|member| member.addresses),
        resolver,
        ipv4_range: params.ipv4,
        ipv6_range: params.ipv6,
        peers,
        routes,
        filter,
        warnings,
    })
}

/// Members in admission order: by the admitting op's lamport clock, then its id.
///
/// Both come from the ledger's deterministic view, so every device agrees on the order.
/// The node id breaks ties only defensively; one op admits one device.
fn admission_order(view: &NetworkView) -> Vec<NodeId> {
    let mut order: Vec<(u64, kith_types::OpId, NodeId)> = view
        .devices()
        .iter()
        .map(|(node, device)| (device.admitted_at_lamport, device.admitted_by, *node))
        .collect();
    order.sort_unstable();
    order.into_iter().map(|(_, _, node)| node).collect()
}

/// The device's tags that are valid policy tags, warning about the rest.
fn valid_tags(node: NodeId, device: &Device, warnings: &mut Vec<Warning>) -> BTreeSet<TagName> {
    device
        .tags
        .iter()
        .filter_map(|tag| {
            TagName::new(tag)
                .map_err(|_| {
                    warnings.push(Warning::InvalidTag {
                        device: node,
                        tag: tag.clone(),
                    });
                })
                .ok()
        })
        .collect()
}

/// The network's policy; see [`records::POLICY`] for the fallbacks.
fn policy(view: &NetworkView, warnings: &mut Vec<Warning>) -> Policy {
    match view.records().get(records::POLICY) {
        None => Policy::default_allow_all(),
        Some(bytes) => Policy::from_bytes(bytes).unwrap_or_else(|error| {
            warnings.push(Warning::InvalidPolicy(error));
            Policy::deny_all()
        }),
    }
}

/// The valid route advertisements by members.
fn routes(view: &NetworkView, warnings: &mut Vec<Warning>) -> BTreeMap<IpPrefix, NodeId> {
    let mut routes = BTreeMap::new();
    let route_records = view
        .records()
        .range::<str, _>((Bound::Included(records::ROUTE_PREFIX), Bound::Unbounded))
        .take_while(|(key, _)| key.starts_with(records::ROUTE_PREFIX));
    for (key, value) in route_records {
        match records::parse_route(key, value).flatten() {
            None => warnings.push(Warning::InvalidRoute { key: key.clone() }),
            Some((prefix, device)) if !view.is_member(&device) => {
                warnings.push(Warning::RouteFromNonMember { prefix, device });
            }
            Some((prefix, device)) => {
                routes.insert(prefix, device);
            }
        }
    }
    routes
}
