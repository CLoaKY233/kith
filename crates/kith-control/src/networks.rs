//! The set of networks this device has joined.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use kith_ledger::NetworkView;
use kith_types::{IpPrefix, NetworkId, NodeId};

use crate::netmap::{ControlError, NetMap, NetMapDiff, build_netmap};

/// Two joined networks whose mesh ranges overlap.
///
/// The router maps a destination address to one network, so it rejects the second
/// network's routes; this warns before that happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Conflict {
    /// The network with the lower id.
    pub first: NetworkId,
    /// The network with the higher id.
    pub second: NetworkId,
    /// The first network's range that overlaps the second's.
    pub range: IpPrefix,
}

/// One [`NetMap`] per joined network.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Networks {
    maps: BTreeMap<NetworkId, NetMap>,
}

impl Networks {
    /// No networks.
    pub const fn new() -> Self {
        Self {
            maps: BTreeMap::new(),
        }
    }

    /// Rebuilds the map of `view`'s network as seen by `local`, returning what changed.
    ///
    /// A network seen for the first time reports every peer as added and everything else
    /// as changed. On error the previous map is kept.
    pub fn update(
        &mut self,
        view: &NetworkView,
        local: NodeId,
    ) -> Result<NetMapDiff, ControlError> {
        let new = build_netmap(view, local)?;
        let diff = match self.maps.get(&new.network_id()) {
            Some(old) => NetMap::diff(old, &new),
            None => new.initial_diff(),
        };
        self.maps.insert(new.network_id(), new);
        Ok(diff)
    }

    /// Forgets a network, returning its last map.
    pub fn remove(&mut self, network: &NetworkId) -> Option<NetMap> {
        self.maps.remove(network)
    }

    /// The map of one network.
    pub fn get(&self, network: &NetworkId) -> Option<&NetMap> {
        self.maps.get(network)
    }

    /// Every network's map, ordered by network id.
    pub fn iter(&self) -> impl Iterator<Item = (&NetworkId, &NetMap)> {
        self.maps.iter()
    }

    /// Every pair of networks whose IPv4 or IPv6 mesh ranges overlap, sorted.
    ///
    /// Quadratic in the number of networks, which is a handful.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let maps: Vec<&NetMap> = self.maps.values().collect();
        let mut conflicts = Vec::new();
        for (index, first) in maps.iter().enumerate() {
            for second in maps.iter().skip(index.saturating_add(1)) {
                let (a4, b4) = (first.ipv4_range(), second.ipv4_range());
                let (a6, b6) = (first.ipv6_range(), second.ipv6_range());
                let overlapping = [
                    (a4.covers(&b4) || b4.covers(&a4)).then_some(IpPrefix::V4(a4)),
                    (a6.covers(&b6) || b6.covers(&a6)).then_some(IpPrefix::V6(a6)),
                ];
                conflicts.extend(overlapping.into_iter().flatten().map(|range| Conflict {
                    first: first.network_id(),
                    second: second.network_id(),
                    range,
                }));
            }
        }
        conflicts
    }
}
