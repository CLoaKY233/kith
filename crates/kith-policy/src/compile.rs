//! Compiling a policy into the inbound packet filter of one device.
//!
//! Policy evaluation happens once per membership or policy change; packet checks happen
//! millions of times a second. So compilation does all the set work up front (resolving
//! tags and devices to host addresses, merging duplicates) and leaves the hot path a
//! binary search over a sorted array plus a scan of a handful of grants.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NodeId};

use crate::allocate::Addresses;
use crate::policy::{PacketProtocol, Policy, Protocol, Selector};
use crate::ports::PortSet;
use crate::tag::TagName;

/// What compilation needs to know about one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The member's tags.
    pub tags: BTreeSet<TagName>,
    /// The member's mesh addresses.
    pub addresses: Addresses,
}

/// The network's members, as seen by the compiler.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Members(BTreeMap<NodeId, Member>);

impl Members {
    /// No members.
    #[must_use]
    pub const fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// Adds or replaces a member, returning the previous entry.
    pub fn insert(&mut self, node: NodeId, member: Member) -> Option<Member> {
        self.0.insert(node, member)
    }

    /// The member `node`, if present.
    #[must_use]
    pub fn get(&self, node: &NodeId) -> Option<&Member> {
        self.0.get(node)
    }

    /// Every member, ordered by node.
    pub fn iter(&self) -> impl Iterator<Item = (&NodeId, &Member)> {
        self.0.iter()
    }

    /// The number of members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no members.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(NodeId, Member)> for Members {
    fn from_iter<I: IntoIterator<Item = (NodeId, Member)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Whether `selector` names the member `node`.
fn selects(selector: &Selector, node: &NodeId, member: &Member) -> bool {
    match selector {
        Selector::Any => true,
        Selector::Device(device) => device == node,
        Selector::Tag(tag) => member.tags.contains(tag),
    }
}

/// One compiled allowance: traffic of `protocol` to `ports` from any of `sources`.
///
/// Invariant: `sources` is sorted, deduplicated and holds only host prefixes (/32, /128).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterEntry {
    sources: Vec<IpPrefix>,
    protocol: Protocol,
    ports: PortSet,
}

impl FilterEntry {
    /// The allowed source hosts, sorted.
    #[must_use]
    pub fn sources(&self) -> &[IpPrefix] {
        &self.sources
    }

    /// The protocol allowed.
    #[must_use]
    pub const fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// The destination ports allowed.
    #[must_use]
    pub const fn ports(&self) -> &PortSet {
        &self.ports
    }

    fn admits(&self, protocol: PacketProtocol, port: Option<u16>) -> bool {
        self.protocol.matches(protocol) && self.ports.matches(port)
    }
}

/// One source address in the lookup index, pointing at its run in `grants`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot<A> {
    source: A,
    start: usize,
    end: usize,
}

/// The inbound filter for one device.
///
/// Built by [`compile`]. Deterministic: equal policies and members give equal filters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompiledFilter {
    local: Option<Addresses>,
    entries: Vec<FilterEntry>,
    /// Sorted by source, one slot per distinct source.
    v4: Vec<Slot<Ipv4Addr>>,
    /// Sorted by source, one slot per distinct source.
    v6: Vec<Slot<Ipv6Addr>>,
    /// Indices into `entries`, grouped per slot.
    grants: Vec<usize>,
}

impl CompiledFilter {
    /// The merged allowances, in a deterministic order.
    #[must_use]
    pub fn entries(&self) -> &[FilterEntry] {
        &self.entries
    }

    /// The local device's own addresses, whose traffic is always allowed.
    #[must_use]
    pub const fn local_addresses(&self) -> Option<Addresses> {
        self.local
    }

    /// Whether an inbound packet from `source` with `protocol` and destination port
    /// `destination_port` (`None` for port-less protocols) is allowed.
    ///
    /// Never allocates. Cost is `O(log s + g · log r)` for `s` distinct allowed sources,
    /// `g` entries that name this source (bounded by the distinct protocol and port
    /// combinations in the policy) and `r` ≤ 64 port ranges per entry.
    #[must_use]
    pub fn allows(
        &self,
        source: IpAddr,
        protocol: PacketProtocol,
        destination_port: Option<u16>,
    ) -> bool {
        let slot = match source {
            IpAddr::V4(source) => {
                if self.local.is_some_and(|local| local.v4 == source) {
                    return true;
                }
                find(&self.v4, &source)
            }
            IpAddr::V6(source) => {
                if self.local.is_some_and(|local| local.v6 == source) {
                    return true;
                }
                find(&self.v6, &source)
            }
        };
        slot.and_then(|(start, end)| self.grants.get(start..end))
            .unwrap_or_default()
            .iter()
            .filter_map(|&index| self.entries.get(index))
            .any(|entry| entry.admits(protocol, destination_port))
    }
}

fn find<A: Ord>(slots: &[Slot<A>], source: &A) -> Option<(usize, usize)> {
    let index = slots
        .binary_search_by(|slot| slot.source.cmp(source))
        .ok()?;
    slots.get(index).map(|slot| (slot.start, slot.end))
}

/// Compiles the rules that let traffic *into* `local`.
///
/// A rule contributes when one of its targets selects `local`: every member matching one
/// of its sources may then send that target's protocol and ports. Traffic from `local`'s
/// own addresses is always allowed; any source that is not a member's address is denied.
/// If `local` is not in `members`, nothing at all is allowed.
#[must_use]
pub fn compile(policy: &Policy, members: &Members, local: NodeId) -> CompiledFilter {
    let Some(local_member) = members.get(&local) else {
        return CompiledFilter::default();
    };

    // (protocol, ports) → allowed source hosts. Merges every rule that opens the same
    // traffic into one entry, and the BTree ordering makes the output deterministic.
    let mut grouped: BTreeMap<(Protocol, &PortSet), BTreeSet<IpPrefix>> = BTreeMap::new();
    for rule in policy.rules() {
        let mut sources: Option<BTreeSet<IpPrefix>> = None;
        for target in rule.destinations() {
            if !selects(target.selector(), &local, local_member) {
                continue;
            }
            // Resolve the rule's sources once, on the first target that applies.
            let hosts = sources.get_or_insert_with(|| {
                members
                    .iter()
                    .filter(|(node, member)| {
                        rule.sources()
                            .iter()
                            .any(|selector| selects(selector, node, member))
                    })
                    .flat_map(|(_, member)| host_prefixes(member.addresses))
                    .collect()
            });
            grouped
                .entry((target.protocol(), target.ports()))
                .or_default()
                .extend(hosts.iter().copied());
        }
    }

    // A source that may send everything needs no narrower entry.
    let everything = (Protocol::Any, &PortSet::All);
    if let Some(unrestricted) = grouped.get(&everything).cloned() {
        for (key, sources) in &mut grouped {
            if *key != everything {
                sources.retain(|source| !unrestricted.contains(source));
            }
        }
    }

    let entries: Vec<FilterEntry> = grouped
        .into_iter()
        .filter(|(_, sources)| !sources.is_empty())
        .map(|((protocol, ports), sources)| FilterEntry {
            sources: sources.into_iter().collect(),
            protocol,
            ports: ports.clone(),
        })
        .collect();

    let (v4, v6, grants) = index(&entries);
    CompiledFilter {
        local: Some(local_member.addresses),
        entries,
        v4,
        v6,
        grants,
    }
}

type Index = (Vec<Slot<Ipv4Addr>>, Vec<Slot<Ipv6Addr>>, Vec<usize>);

/// Builds the per-source lookup tables.
fn index(entries: &[FilterEntry]) -> Index {
    let mut by_v4: BTreeMap<Ipv4Addr, Vec<usize>> = BTreeMap::new();
    let mut by_v6: BTreeMap<Ipv6Addr, Vec<usize>> = BTreeMap::new();
    for (position, entry) in entries.iter().enumerate() {
        for source in &entry.sources {
            match source {
                IpPrefix::V4(prefix) => by_v4.entry(prefix.addr()).or_default().push(position),
                IpPrefix::V6(prefix) => by_v6.entry(prefix.addr()).or_default().push(position),
            }
        }
    }
    let mut grants = Vec::new();
    let v4 = flatten(by_v4, &mut grants);
    let v6 = flatten(by_v6, &mut grants);
    (v4, v6, grants)
}

fn flatten<A>(table: BTreeMap<A, Vec<usize>>, grants: &mut Vec<usize>) -> Vec<Slot<A>> {
    table
        .into_iter()
        .map(|(source, positions)| {
            let start = grants.len();
            grants.extend(positions);
            Slot {
                source,
                start,
                end: grants.len(),
            }
        })
        .collect()
}

/// A member's addresses as host prefixes.
fn host_prefixes(addresses: Addresses) -> impl Iterator<Item = IpPrefix> {
    let v4 = Ipv4Prefix::new(addresses.v4, Ipv4Prefix::WIDTH)
        .ok()
        .map(IpPrefix::V4);
    let v6 = Ipv6Prefix::new(addresses.v6, Ipv6Prefix::WIDTH)
        .ok()
        .map(IpPrefix::V6);
    v4.into_iter().chain(v6)
}

#[cfg(test)]
mod tests;
