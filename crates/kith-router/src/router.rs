//! The multi-network router: destination IP to `(NetworkId, NodeId)`.
//!
//! All networks share one [`PrefixTable`], because one TUN carries every network (see
//! `docs/architecture.md`). Sharing a table is only sound if no network can silently shadow
//! another, so [`Router::set_network`] enforces the rules below before it changes anything:
//! a rejected update leaves the router exactly as it was.
//!
//! # Conflict rules
//!
//! Each network has an IPv4 and an IPv6 [mesh range](MeshRanges), the block its peers'
//! addresses come from. Its routes are either **host routes** (`/32` or `/128`, a peer's own
//! address) or **subnet routes** (anything shorter, advertised by a peer).
//!
//! 1. **Mesh ranges are exclusive.** A network's mesh range must not overlap another
//!    network's mesh range of the same family in any way: equal, nested or partial.
//!    Violations are a [`ConflictError`] of kind [`ConflictKind::MeshRanges`].
//! 2. **Host routes stay home.** Every host route must lie inside its own network's mesh
//!    range, or [`SetNetworkError::HostOutsideMesh`].
//! 3. **Subnet routes keep out of other meshes.** A subnet route must not overlap another
//!    network's mesh range (inside it, or covering it), and a new mesh range must not overlap
//!    another network's subnet routes: [`ConflictKind::RouteInMeshRange`].
//! 4. **Subnet routes may nest.** Outside the mesh ranges, subnet routes of different
//!    networks may overlap, and longest-prefix match decides. Two networks holding the
//!    identical prefix is still a conflict: [`ConflictKind::SamePrefix`].
//!
//! Rules 1 and 2 together mean host routes of different networks can never collide, and
//! rule 3 means no other network can capture traffic for a mesh address.
//!
//! The default routes `0.0.0.0/0` and `::/0` are never accepted as ordinary routes. They
//! belong to the exit-node mechanism: each network may name one exit peer
//! ([`Router::set_exit_peer`]) and at most one network is the active exit
//! ([`Router::set_active_exit`]). The exit is consulted only when no specific route in any
//! network matches.

use alloc::collections::BTreeMap;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NetworkId, NodeId};

use crate::PrefixTable;

/// Where a packet goes: the network, the peer in it, and the prefix that matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// The network the peer belongs to.
    pub network: NetworkId,
    /// The peer to send to.
    pub peer: NodeId,
    /// The matching prefix; `0.0.0.0/0` or `::/0` for the exit route.
    pub prefix: IpPrefix,
}

/// A network's IPv4 and IPv6 mesh ranges: the blocks its peers' addresses come from.
///
/// Mesh ranges of different networks never overlap (see the [module documentation](self)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshRanges {
    /// The IPv4 mesh range, e.g. `100.64.0.0/16`.
    pub v4: Ipv4Prefix,
    /// The IPv6 mesh range, e.g. `fd7a:115c:a1e0::/48`.
    pub v6: Ipv6Prefix,
}

impl MeshRanges {
    /// The mesh range of `prefix`'s family.
    fn for_family(&self, prefix: &IpPrefix) -> IpPrefix {
        match prefix {
            IpPrefix::V4(_) => IpPrefix::V4(self.v4),
            IpPrefix::V6(_) => IpPrefix::V6(self.v6),
        }
    }

    fn both(&self) -> [IpPrefix; 2] {
        [IpPrefix::V4(self.v4), IpPrefix::V6(self.v6)]
    }
}

/// Which conflict rule was broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConflictKind {
    /// Two mesh ranges overlap.
    MeshRanges,
    /// A subnet route overlaps another network's mesh range.
    RouteInMeshRange,
    /// Two networks hold the identical route prefix.
    SamePrefix,
}

/// Two networks claim address space in a way the conflict rules forbid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "network {network} prefix {prefix} conflicts with network {existing_network} prefix \
     {existing_prefix} ({kind:?})"
)]
pub struct ConflictError {
    /// Which rule was broken.
    pub kind: ConflictKind,
    /// The network being set.
    pub network: NetworkId,
    /// Its conflicting mesh range or route.
    pub prefix: IpPrefix,
    /// The network that already holds the other prefix.
    pub existing_network: NetworkId,
    /// That network's conflicting mesh range or route.
    pub existing_prefix: IpPrefix,
}

/// Why [`Router::set_network`] rejected a route set. The router is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SetNetworkError {
    /// The routes overlap another network's in a way the conflict rule forbids.
    #[error(transparent)]
    Conflict(#[from] ConflictError),
    /// A default route was given; use the exit-node API instead.
    #[error("default route {0} must be set as an exit peer, not a route")]
    DefaultRoute(IpPrefix),
    /// A host route lies outside its own network's mesh range.
    #[error("host route {route} is outside the mesh range {mesh}")]
    HostOutsideMesh {
        /// The host route.
        route: IpPrefix,
        /// The network's mesh range of that family.
        mesh: IpPrefix,
    },
    /// The same prefix was listed twice with different peers.
    #[error("prefix {0} is listed with two different peers")]
    DuplicatePrefix(IpPrefix),
}

/// Why an exit-node change was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ExitError {
    /// The network is not known to the router.
    #[error("unknown network {0}")]
    UnknownNetwork(NetworkId),
    /// The network has no exit peer to route through.
    #[error("network {0} has no exit peer")]
    NoExitPeer(NetworkId),
}

/// The resolved active exit route.
#[derive(Debug, Clone, Copy)]
struct Exit {
    network: NetworkId,
    peer: NodeId,
    v4: IpPrefix,
    v6: IpPrefix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Network {
    mesh: MeshRanges,
    routes: BTreeMap<IpPrefix, NodeId>,
    exit_peer: Option<NodeId>,
}

/// Routes destination addresses across every joined network.
///
/// Updates may allocate; [`Router::route`] never does.
#[derive(Debug, Clone, Default)]
pub struct Router {
    networks: BTreeMap<NetworkId, Network>,
    table: PrefixTable<(NetworkId, NodeId)>,
    active_exit: Option<NetworkId>,
    /// The active exit, resolved, so lookup reads no map.
    exit: Option<Exit>,
}

impl Router {
    /// A router with no networks.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds network `id`, or replaces its mesh ranges and all of its routes. Its exit peer is
    /// kept.
    ///
    /// Fails, changing nothing, if a route is a default route, a prefix is listed twice with
    /// different peers, or any conflict rule in the [module documentation](self) is broken.
    pub fn set_network(
        &mut self,
        id: NetworkId,
        mesh: MeshRanges,
        routes: impl IntoIterator<Item = (IpPrefix, NodeId)>,
    ) -> Result<(), SetNetworkError> {
        let mut wanted = BTreeMap::new();
        for (prefix, peer) in routes {
            if prefix.prefix_len() == 0 {
                return Err(SetNetworkError::DefaultRoute(prefix));
            }
            if wanted.insert(prefix, peer).is_some_and(|old| old != peer) {
                return Err(SetNetworkError::DuplicatePrefix(prefix));
            }
        }
        self.check(id, &mesh, &wanted)?;

        let exit_peer = match self.networks.remove(&id) {
            Some(old) => {
                for prefix in old.routes.keys() {
                    self.table.remove(prefix);
                }
                old.exit_peer
            }
            None => None,
        };
        for (prefix, peer) in &wanted {
            self.table.insert(*prefix, (id, *peer));
        }
        self.networks.insert(
            id,
            Network {
                mesh,
                routes: wanted,
                exit_peer,
            },
        );
        Ok(())
    }

    /// Applies the conflict rules to a proposed configuration of network `id`.
    fn check(
        &self,
        id: NetworkId,
        mesh: &MeshRanges,
        routes: &BTreeMap<IpPrefix, NodeId>,
    ) -> Result<(), SetNetworkError> {
        let conflict = |kind, prefix, existing_network, existing_prefix| ConflictError {
            kind,
            network: id,
            prefix,
            existing_network,
            existing_prefix,
        };
        for (&other_id, other) in self
            .networks
            .iter()
            .filter(|(other_id, _)| **other_id != id)
        {
            // Rule 1: mesh ranges are exclusive.
            for (ours, theirs) in mesh.both().into_iter().zip(other.mesh.both()) {
                if overlaps(&ours, &theirs) {
                    return Err(conflict(ConflictKind::MeshRanges, ours, other_id, theirs).into());
                }
            }
            // Rule 3, both directions: no subnet route overlaps another network's mesh.
            for theirs in other.routes.keys() {
                let ours = mesh.for_family(theirs);
                if overlaps(&ours, theirs) {
                    return Err(
                        conflict(ConflictKind::RouteInMeshRange, ours, other_id, *theirs).into(),
                    );
                }
            }
            for ours in routes.keys() {
                let theirs = other.mesh.for_family(ours);
                if overlaps(ours, &theirs) {
                    return Err(
                        conflict(ConflictKind::RouteInMeshRange, *ours, other_id, theirs).into(),
                    );
                }
            }
        }
        for route in routes.keys() {
            let own = mesh.for_family(route);
            // Rule 2: host routes stay inside their own mesh range.
            if is_host(route) && !overlaps(&own, route) {
                return Err(SetNetworkError::HostOutsideMesh {
                    route: *route,
                    mesh: own,
                });
            }
            // Rule 4: identical prefixes in two networks conflict.
            if let Some(&(owner, _)) = self.table.get(route)
                && owner != id
            {
                return Err(conflict(ConflictKind::SamePrefix, *route, owner, *route).into());
            }
        }
        Ok(())
    }

    /// Removes network `id` with all its routes and its exit peer. If it was the active exit,
    /// no exit remains active. Returns whether the network existed.
    pub fn remove_network(&mut self, id: NetworkId) -> bool {
        let Some(network) = self.networks.remove(&id) else {
            return false;
        };
        for prefix in network.routes.keys() {
            self.table.remove(prefix);
        }
        if self.active_exit == Some(id) {
            self.active_exit = None;
        }
        self.refresh_exit();
        true
    }

    /// Sets or clears the exit peer of network `id`. Clearing the active exit's peer
    /// deactivates it.
    pub fn set_exit_peer(&mut self, id: NetworkId, peer: Option<NodeId>) -> Result<(), ExitError> {
        let network = self
            .networks
            .get_mut(&id)
            .ok_or(ExitError::UnknownNetwork(id))?;
        network.exit_peer = peer;
        if peer.is_none() && self.active_exit == Some(id) {
            self.active_exit = None;
        }
        self.refresh_exit();
        Ok(())
    }

    /// Makes network `id` the one active exit, replacing any other, or with `None` stops
    /// using an exit node.
    pub fn set_active_exit(&mut self, id: Option<NetworkId>) -> Result<(), ExitError> {
        if let Some(id) = id {
            let network = self
                .networks
                .get(&id)
                .ok_or(ExitError::UnknownNetwork(id))?;
            if network.exit_peer.is_none() {
                return Err(ExitError::NoExitPeer(id));
            }
        }
        self.active_exit = id;
        self.refresh_exit();
        Ok(())
    }

    /// The active exit network, if any.
    pub const fn active_exit(&self) -> Option<NetworkId> {
        self.active_exit
    }

    /// Where to send a packet for `dst`: the longest specific route in any network, else the
    /// active exit's default route, else `None`. Never allocates.
    pub fn route(&self, dst: IpAddr) -> Option<Route> {
        if let Some((prefix, &(network, peer))) = self.table.lookup(dst) {
            return Some(Route {
                network,
                peer,
                prefix: *prefix,
            });
        }
        let exit = self.exit.as_ref()?;
        let prefix = match dst {
            IpAddr::V4(_) => exit.v4,
            IpAddr::V6(_) => exit.v6,
        };
        Some(Route {
            network: exit.network,
            peer: exit.peer,
            prefix,
        })
    }

    fn refresh_exit(&mut self) {
        self.exit = self.active_exit.and_then(|id| {
            let peer = self.networks.get(&id)?.exit_peer?;
            Some(Exit {
                network: id,
                peer,
                v4: Ipv4Prefix::new(Ipv4Addr::UNSPECIFIED, 0).ok()?.into(),
                v6: Ipv6Prefix::new(Ipv6Addr::UNSPECIFIED, 0).ok()?.into(),
            })
        });
    }
}

/// Whether two prefixes share any address. Prefixes of different families never do.
fn overlaps(a: &IpPrefix, b: &IpPrefix) -> bool {
    match (a, b) {
        (IpPrefix::V4(a), IpPrefix::V4(b)) => a.covers(b) || b.covers(a),
        (IpPrefix::V6(a), IpPrefix::V6(b)) => a.covers(b) || b.covers(a),
        (IpPrefix::V4(_), IpPrefix::V6(_)) | (IpPrefix::V6(_), IpPrefix::V4(_)) => false,
    }
}

/// Whether `prefix` is a single address.
fn is_host(prefix: &IpPrefix) -> bool {
    match prefix {
        IpPrefix::V4(p) => p.prefix_len() == Ipv4Prefix::WIDTH,
        IpPrefix::V6(p) => p.prefix_len() == Ipv6Prefix::WIDTH,
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::*;

    fn p(text: &str) -> IpPrefix {
        text.parse().unwrap()
    }

    fn a(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn net(byte: u8) -> NetworkId {
        NetworkId::from_bytes([byte; 32])
    }

    fn node(byte: u8) -> NodeId {
        NodeId::from_bytes([byte; 32])
    }

    fn mesh(v4: &str, v6: &str) -> MeshRanges {
        MeshRanges {
            v4: v4.parse().unwrap(),
            v6: v6.parse().unwrap(),
        }
    }

    /// Mesh ranges for network `n`: `100.64.n.0/24` and `fd7a:n::/32`.
    fn mesh_of(n: u8) -> MeshRanges {
        mesh(
            &alloc::format!("100.64.{n}.0/24"),
            &alloc::format!("fd7a:{n}::/32"),
        )
    }

    fn hop(router: &Router, addr: &str) -> Option<(NetworkId, NodeId, IpPrefix)> {
        router
            .route(a(addr))
            .map(|route| (route.network, route.peer, route.prefix))
    }

    fn two_networks() -> Router {
        let mut router = Router::new();
        router
            .set_network(
                net(1),
                mesh_of(1),
                vec![
                    (p("100.64.1.1/32"), node(1)),
                    (p("192.168.1.0/24"), node(2)),
                ],
            )
            .unwrap();
        router
            .set_network(
                net(2),
                mesh_of(2),
                vec![(p("100.64.2.1/32"), node(3)), (p("fd7a:2::3/128"), node(3))],
            )
            .unwrap();
        router
    }

    #[test]
    fn routes_across_networks() {
        let router = two_networks();
        assert_eq!(
            hop(&router, "100.64.1.1"),
            Some((net(1), node(1), p("100.64.1.1/32")))
        );
        assert_eq!(
            hop(&router, "100.64.2.1"),
            Some((net(2), node(3), p("100.64.2.1/32")))
        );
        assert_eq!(
            hop(&router, "192.168.1.77"),
            Some((net(1), node(2), p("192.168.1.0/24")))
        );
        assert_eq!(
            hop(&router, "fd7a:2::3"),
            Some((net(2), node(3), p("fd7a:2::3/128")))
        );
        assert_eq!(
            hop(&router, "100.64.1.2"),
            None,
            "a mesh range alone routes nowhere"
        );
        assert_eq!(hop(&router, "8.8.8.8"), None);
    }

    #[test]
    fn replacing_a_network_drops_its_old_routes() {
        let mut router = two_networks();
        router
            .set_network(net(1), mesh_of(1), vec![(p("10.0.0.0/8"), node(9))])
            .unwrap();
        assert_eq!(hop(&router, "100.64.1.1"), None);
        assert_eq!(
            hop(&router, "10.1.1.1"),
            Some((net(1), node(9), p("10.0.0.0/8")))
        );
        assert!(router.remove_network(net(1)));
        assert!(!router.remove_network(net(1)));
        assert_eq!(hop(&router, "10.1.1.1"), None);
        assert_eq!(
            hop(&router, "100.64.2.1"),
            Some((net(2), node(3), p("100.64.2.1/32")))
        );
    }

    #[test]
    fn overlapping_mesh_ranges_conflict() {
        let mut router = two_networks();
        for (v4, v6, ours, theirs) in [
            (
                "100.64.1.0/24",
                "fd7a:9::/32",
                "100.64.1.0/24",
                "100.64.1.0/24",
            ), // equal
            (
                "100.64.1.128/25",
                "fd7a:9::/32",
                "100.64.1.128/25",
                "100.64.1.0/24",
            ), // nested
            (
                "100.64.0.0/16",
                "fd7a:9::/32",
                "100.64.0.0/16",
                "100.64.1.0/24",
            ), // covering
            ("100.64.9.0/24", "fd7a::/16", "fd7a::/16", "fd7a:1::/32"), // IPv6
        ] {
            let error = router
                .set_network(net(3), mesh(v4, v6), Vec::new())
                .unwrap_err();
            assert_eq!(
                error,
                SetNetworkError::Conflict(ConflictError {
                    kind: ConflictKind::MeshRanges,
                    network: net(3),
                    prefix: p(ours),
                    existing_network: net(1),
                    existing_prefix: p(theirs),
                })
            );
        }
    }

    #[test]
    fn host_routes_must_be_inside_their_own_mesh() {
        let mut router = two_networks();
        let error = router
            .set_network(net(3), mesh_of(3), vec![(p("100.64.9.1/32"), node(4))])
            .unwrap_err();
        assert_eq!(
            error,
            SetNetworkError::HostOutsideMesh {
                route: p("100.64.9.1/32"),
                mesh: p("100.64.3.0/24")
            }
        );
        let error = router
            .set_network(net(3), mesh_of(3), vec![(p("fd7a:1::1/128"), node(4))])
            .unwrap_err();
        assert!(matches!(
            error,
            SetNetworkError::Conflict(ConflictError {
                kind: ConflictKind::RouteInMeshRange,
                ..
            })
        ));
    }

    #[test]
    fn subnet_routes_must_not_touch_another_mesh() {
        let mut router = two_networks();
        // Inside another network's mesh.
        let error = router
            .set_network(net(3), mesh_of(3), vec![(p("100.64.1.0/28"), node(4))])
            .unwrap_err();
        assert_eq!(
            error,
            SetNetworkError::Conflict(ConflictError {
                kind: ConflictKind::RouteInMeshRange,
                network: net(3),
                prefix: p("100.64.1.0/28"),
                existing_network: net(1),
                existing_prefix: p("100.64.1.0/24"),
            })
        );
        // Covering another network's mesh.
        let error = router
            .set_network(net(3), mesh_of(3), vec![(p("100.0.0.0/8"), node(4))])
            .unwrap_err();
        assert!(matches!(
            error,
            SetNetworkError::Conflict(ConflictError {
                kind: ConflictKind::RouteInMeshRange,
                ..
            })
        ));
        // A new mesh range landing on an existing subnet route.
        let error = router
            .set_network(net(3), mesh("192.168.1.0/25", "fd7a:3::/32"), Vec::new())
            .unwrap_err();
        assert_eq!(
            error,
            SetNetworkError::Conflict(ConflictError {
                kind: ConflictKind::RouteInMeshRange,
                network: net(3),
                prefix: p("192.168.1.0/25"),
                existing_network: net(1),
                existing_prefix: p("192.168.1.0/24"),
            })
        );
    }

    #[test]
    fn identical_subnet_routes_conflict() {
        let mut router = two_networks();
        let error = router
            .set_network(net(2), mesh_of(2), vec![(p("192.168.1.0/24"), node(4))])
            .unwrap_err();
        assert_eq!(
            error,
            SetNetworkError::Conflict(ConflictError {
                kind: ConflictKind::SamePrefix,
                network: net(2),
                prefix: p("192.168.1.0/24"),
                existing_network: net(1),
                existing_prefix: p("192.168.1.0/24"),
            })
        );
        // The failed update changed nothing.
        assert_eq!(
            hop(&router, "100.64.2.1"),
            Some((net(2), node(3), p("100.64.2.1/32")))
        );
    }

    #[test]
    fn nested_subnet_routes_are_decided_by_longest_match() {
        let mut router = two_networks();
        router
            .set_network(net(3), mesh_of(3), vec![(p("192.168.1.0/28"), node(5))])
            .unwrap();
        assert_eq!(
            hop(&router, "192.168.1.5"),
            Some((net(3), node(5), p("192.168.1.0/28")))
        );
        assert_eq!(
            hop(&router, "192.168.1.66"),
            Some((net(1), node(2), p("192.168.1.0/24")))
        );
        router.remove_network(net(3));
        assert_eq!(
            hop(&router, "192.168.1.5"),
            Some((net(1), node(2), p("192.168.1.0/24")))
        );
    }

    #[test]
    fn bad_route_sets_are_rejected() {
        let mut router = Router::new();
        assert_eq!(
            router.set_network(net(1), mesh_of(1), vec![(p("0.0.0.0/0"), node(1))]),
            Err(SetNetworkError::DefaultRoute(p("0.0.0.0/0")))
        );
        assert_eq!(
            router.set_network(
                net(1),
                mesh_of(1),
                vec![(p("10.0.0.0/8"), node(1)), (p("10.0.0.0/8"), node(2))]
            ),
            Err(SetNetworkError::DuplicatePrefix(p("10.0.0.0/8")))
        );
        assert_eq!(hop(&router, "10.0.0.1"), None);
    }

    #[test]
    fn exit_is_the_last_resort() {
        let mut router = two_networks();
        assert_eq!(
            router.set_active_exit(Some(net(2))),
            Err(ExitError::NoExitPeer(net(2)))
        );
        assert_eq!(
            router.set_exit_peer(net(9), Some(node(1))),
            Err(ExitError::UnknownNetwork(net(9)))
        );
        router.set_exit_peer(net(2), Some(node(3))).unwrap();
        router.set_exit_peer(net(1), Some(node(1))).unwrap();
        router.set_active_exit(Some(net(2))).unwrap();
        assert_eq!(
            hop(&router, "8.8.8.8"),
            Some((net(2), node(3), p("0.0.0.0/0")))
        );
        assert_eq!(
            hop(&router, "2001:db8::1"),
            Some((net(2), node(3), p("::/0")))
        );
        // A specific route in any network, including a non-exit one, wins over the exit.
        assert_eq!(
            hop(&router, "192.168.1.1"),
            Some((net(1), node(2), p("192.168.1.0/24")))
        );
        // Only one exit is active at a time.
        router.set_active_exit(Some(net(1))).unwrap();
        assert_eq!(
            hop(&router, "8.8.8.8"),
            Some((net(1), node(1), p("0.0.0.0/0")))
        );
        router.set_active_exit(None).unwrap();
        assert_eq!(hop(&router, "8.8.8.8"), None);
    }

    #[test]
    fn exit_survives_route_updates_and_dies_with_its_network() {
        let mut router = two_networks();
        router.set_exit_peer(net(2), Some(node(3))).unwrap();
        router.set_active_exit(Some(net(2))).unwrap();
        router.set_network(net(2), mesh_of(2), Vec::new()).unwrap();
        assert_eq!(
            hop(&router, "100.64.2.1"),
            Some((net(2), node(3), p("0.0.0.0/0")))
        );
        router.set_exit_peer(net(2), None).unwrap();
        assert_eq!(router.active_exit(), None);
        router.set_exit_peer(net(2), Some(node(3))).unwrap();
        router.set_active_exit(Some(net(2))).unwrap();
        router.remove_network(net(2));
        assert_eq!(router.active_exit(), None);
        assert_eq!(hop(&router, "8.8.8.8"), None);
    }

    /// Everything observable about a router, for "a failure changes nothing" checks.
    type Snapshot = (
        BTreeMap<NetworkId, Network>,
        Vec<(IpPrefix, (NetworkId, NodeId))>,
    );

    fn snapshot(router: &Router) -> Snapshot {
        let table = router
            .table
            .iter()
            .map(|(prefix, value)| (*prefix, *value))
            .collect();
        (router.networks.clone(), table)
    }

    fn v4(base: u32, len: u8) -> Ipv4Prefix {
        Ipv4Prefix::truncating(Ipv4Addr::from(base), len).unwrap()
    }

    fn v6(base: u128, len: u8) -> Ipv6Prefix {
        Ipv6Prefix::truncating(Ipv6Addr::from(base), len).unwrap()
    }

    /// A small address space, so meshes and routes collide often.
    fn arb_mesh() -> impl Strategy<Value = MeshRanges> {
        (0u32..8, 14u8..=26, 0u128..4, 30u8..=40).prop_map(|(a, l4, b, l6)| MeshRanges {
            v4: v4(0x6440_0000 | (a << 14), l4),
            v6: v6((0xfd7a << 112) | (b << 94), l6),
        })
    }

    fn arb_route() -> impl Strategy<Value = (IpPrefix, NodeId)> {
        (
            any::<bool>(),
            0u32..8,
            any::<u16>(),
            prop_oneof![Just(32u8), 8u8..=31],
            0u8..3,
        )
            .prop_map(|(in_mesh_space, a, host, len, peer)| {
                let base = if in_mesh_space {
                    0x6440_0000 | (a << 14)
                } else {
                    0x0a00_0000 | (a << 16)
                };
                (IpPrefix::V4(v4(base | u32::from(host), len)), node(peer))
            })
    }

    #[derive(Debug, Clone)]
    enum Op {
        Set(u8, MeshRanges, Vec<(IpPrefix, NodeId)>),
        Remove(u8),
    }

    fn arb_op() -> impl Strategy<Value = Op> {
        prop_oneof![
            4 => (0u8..4, arb_mesh(), prop::collection::vec(arb_route(), 0..6))
                .prop_map(|(n, mesh, routes)| Op::Set(n, mesh, routes)),
            1 => (0u8..4).prop_map(Op::Remove),
        ]
    }

    fn assert_invariants(router: &Router) {
        let networks: Vec<_> = router.networks.iter().collect();
        for (id, network) in &networks {
            for route in network.routes.keys() {
                if is_host(route) {
                    assert!(overlaps(&network.mesh.for_family(route), route));
                }
            }
            for (other_id, other) in &networks {
                if id == other_id {
                    continue;
                }
                for (ours, theirs) in network.mesh.both().into_iter().zip(other.mesh.both()) {
                    assert!(
                        !overlaps(&ours, &theirs),
                        "mesh ranges {ours} and {theirs} overlap"
                    );
                }
                for route in network.routes.keys() {
                    assert!(!overlaps(route, &other.mesh.for_family(route)));
                    assert!(!other.routes.contains_key(route));
                }
            }
        }
    }

    proptest! {
        #[test]
        fn configurations_are_valid_or_rejected_without_change(
            ops in prop::collection::vec(arb_op(), 1..24),
        ) {
            let mut router = Router::new();
            for op in ops {
                match op {
                    Op::Set(n, mesh, routes) => {
                        let before = snapshot(&router);
                        if router.set_network(net(n), mesh, routes).is_err() {
                            prop_assert_eq!(snapshot(&router), before);
                        }
                    }
                    Op::Remove(n) => {
                        router.remove_network(net(n));
                    }
                }
                assert_invariants(&router);
            }
        }
    }
}
