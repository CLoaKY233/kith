//! Deterministic mesh address allocation.
//!
//! Every device computes every member's addresses itself, from the replicated member list,
//! so no coordinator hands out leases and all devices agree without talking.
//!
//! Each member's first candidate is a BLAKE3 hash of its [`NodeId`] under
//! [`ALLOCATION_CONTEXT`], reduced into the assignable part of the range. If an earlier
//! member already holds that address, the member probes upward one address at a time,
//! wrapping around at the end of the range.
//!
//! # Stability
//!
//! Members are placed in admission order, and a member's address depends only on its own
//! hash and the addresses of members admitted before it. So appending members to the list
//! never moves anyone already in it: existing devices keep their addresses as the network
//! grows.
//!
//! **Caveat:** the order is the ledger's admission order. If two admins admit members
//! concurrently, the ledger's deterministic merge may sort a newly learned member *before*
//! one a device already placed. When both members' candidates collide, the later-sorted one
//! moves. Collisions are rare in practice (and practically impossible in the IPv6 range),
//! but callers must treat addresses as derived state and recompute them after every merge.
//!
//! # Reserved addresses
//!
//! These are never assigned:
//! - offset 0, the network address (for IPv6, the subnet-router anycast address);
//! - offset 1, the mesh DNS resolver, see [`resolver_address`];
//! - for IPv4, the last address, the broadcast address.

use alloc::collections::{BTreeMap, BTreeSet};
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use kith_crypto::{Context, Hash};
use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NodeId};

/// The hash context for an address candidate. Changing it moves every device's address.
pub const ALLOCATION_CONTEXT: Context = Context::new("kith 2026-10 address allocation v1");

/// Offset of the first assignable address: offsets 0 and 1 are reserved.
const FIRST_ASSIGNABLE: u128 = 2;

/// A member's mesh addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Addresses {
    /// The IPv4 address.
    pub v4: Ipv4Addr,
    /// The IPv6 address.
    pub v6: Ipv6Addr,
}

/// An address family, for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// IPv4.
    V4,
    /// IPv6.
    V6,
}

/// Why addresses could not be allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AllocationError {
    /// The range has no assignable address: longer than /30 for IPv4 or /126 for IPv6.
    #[error("{family:?} range /{prefix_len} is too small (at most /{max} allowed)")]
    RangeTooSmall {
        /// The family of the range.
        family: Family,
        /// The prefix length given.
        prefix_len: u8,
        /// The longest usable prefix length.
        max: u8,
    },
    /// Every assignable address in the range is taken.
    #[error("{family:?} range is full")]
    RangeFull {
        /// The family of the full range.
        family: Family,
    },
    /// A node appeared twice in the admission list.
    #[error("{0:?} appears twice in the member list")]
    DuplicateMember(NodeId),
}

/// The geometry of one range: where it starts and how many addresses can be handed out.
#[derive(Debug, Clone, Copy)]
struct Space {
    family: Family,
    base: u128,
    /// Number of assignable addresses, starting at offset [`FIRST_ASSIGNABLE`]. Never zero.
    assignable: u128,
}

impl Space {
    fn new(family: Family, base: u128, width: u8, prefix_len: u8) -> Result<Self, AllocationError> {
        // Reserved offsets: network and resolver, plus broadcast for IPv4.
        let (reserved, max) = match family {
            Family::V4 => (3, 30),
            Family::V6 => (2, 126),
        };
        let too_small = AllocationError::RangeTooSmall {
            family,
            prefix_len,
            max,
        };
        // `host_bits >= 2` guarantees at least one assignable address.
        let host_bits = width
            .checked_sub(prefix_len)
            .filter(|&bits| bits >= 2)
            .ok_or(too_small)?;
        // The highest offset in the range, `2^host_bits - 1`, computed without overflowing
        // when the range is the whole address space.
        let last_offset = u128::MAX >> (128_u8.saturating_sub(host_bits));
        // `last_offset + 1 - reserved`, and `last_offset >= 3 >= reserved - 1`.
        let assignable = last_offset.checked_sub(reserved - 1).ok_or(too_small)?;
        Ok(Self {
            family,
            base,
            assignable,
        })
    }

    fn v4(range: Ipv4Prefix) -> Result<Self, AllocationError> {
        let base = u128::from(u32::from(range.addr()));
        Self::new(Family::V4, base, Ipv4Prefix::WIDTH, range.prefix_len())
    }

    fn v6(range: Ipv6Prefix) -> Result<Self, AllocationError> {
        let base = u128::from(range.addr());
        Self::new(Family::V6, base, Ipv6Prefix::WIDTH, range.prefix_len())
    }

    /// The first assignable offset at or after `seed`'s candidate that is not in `taken`.
    fn place(&self, seed: u128, taken: &BTreeSet<u128>) -> Result<u128, AllocationError> {
        let full = AllocationError::RangeFull {
            family: self.family,
        };
        // `taken` only ever holds assignable offsets, so this proves a free one exists and
        // the probe below terminates.
        if u128::try_from(taken.len()).map_or(true, |count| count >= self.assignable) {
            return Err(full);
        }
        let mut index = seed % self.assignable;
        loop {
            let offset = FIRST_ASSIGNABLE.checked_add(index).ok_or(full)?;
            if !taken.contains(&offset) {
                return Ok(offset);
            }
            index = index.checked_add(1).ok_or(full)? % self.assignable;
        }
    }

    /// The address at `offset`. Offsets are below `2^host_bits`, so OR-ing into the
    /// network address cannot carry into the prefix.
    const fn address(&self, offset: u128) -> u128 {
        self.base | offset
    }
}

/// The address of the mesh DNS resolver in `range`: the first host address.
///
/// Fails if the range is too small to hold any device.
pub fn resolver_address(range: IpPrefix) -> Result<IpAddr, AllocationError> {
    match range {
        IpPrefix::V4(range) => {
            let space = Space::v4(range)?;
            Ok(IpAddr::V4(to_v4(space.address(1))))
        }
        IpPrefix::V6(range) => {
            let space = Space::v6(range)?;
            Ok(IpAddr::V6(Ipv6Addr::from(space.address(1))))
        }
    }
}

/// Assigns IPv4 and IPv6 addresses to every member, listed in admission order.
///
/// Deterministic: the same inputs give the same map on every device. See the module docs
/// for the stability guarantee and its caveat.
pub fn allocate(
    v4: Ipv4Prefix,
    v6: Ipv6Prefix,
    members_in_admission_order: &[NodeId],
) -> Result<BTreeMap<NodeId, Addresses>, AllocationError> {
    let space_v4 = Space::v4(v4)?;
    let space_v6 = Space::v6(v6)?;
    let mut taken_v4 = BTreeSet::new();
    let mut taken_v6 = BTreeSet::new();
    let mut assigned = BTreeMap::new();
    for &node in members_in_admission_order {
        if assigned.contains_key(&node) {
            return Err(AllocationError::DuplicateMember(node));
        }
        let (seed_v4, seed_v6) = seeds(node);
        let offset_v4 = space_v4.place(seed_v4, &taken_v4)?;
        let offset_v6 = space_v6.place(seed_v6, &taken_v6)?;
        taken_v4.insert(offset_v4);
        taken_v6.insert(offset_v6);
        let addresses = Addresses {
            v4: to_v4(space_v4.address(offset_v4)),
            v6: Ipv6Addr::from(space_v6.address(offset_v6)),
        };
        assigned.insert(node, addresses);
    }
    Ok(assigned)
}

/// The two candidate seeds: the first and second halves of the node's hash.
fn seeds(node: NodeId) -> (u128, u128) {
    let digest = Hash::of(ALLOCATION_CONTEXT, node.as_bytes()).to_bytes();
    let mut first = [0_u8; 16];
    let mut second = [0_u8; 16];
    let (head, tail) = digest.split_at(16);
    first.copy_from_slice(head);
    second.copy_from_slice(tail);
    (u128::from_be_bytes(first), u128::from_be_bytes(second))
}

/// An IPv4 address from the low 32 bits. IPv4 offsets and bases always fit.
fn to_v4(bits: u128) -> Ipv4Addr {
    Ipv4Addr::from(u32::try_from(bits).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::collections::BTreeSet;
    use alloc::vec::Vec;
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NodeId};
    use proptest::prelude::*;

    use super::{AllocationError, Family, allocate, resolver_address};

    fn v4(text: &str) -> Ipv4Prefix {
        text.parse().unwrap()
    }

    fn v6(text: &str) -> Ipv6Prefix {
        text.parse().unwrap()
    }

    fn node(index: u8) -> NodeId {
        NodeId::from_bytes([index; 32])
    }

    fn nodes(count: u8) -> Vec<NodeId> {
        (0..count).map(node).collect()
    }

    #[test]
    fn deterministic_and_in_range() {
        let members = nodes(40);
        let first = allocate(v4("100.64.0.0/24"), v6("fd00:1::/64"), &members).unwrap();
        assert_eq!(
            first,
            allocate(v4("100.64.0.0/24"), v6("fd00:1::/64"), &members).unwrap()
        );
        let v4_set: BTreeSet<_> = first.values().map(|a| a.v4).collect();
        let v6_set: BTreeSet<_> = first.values().map(|a| a.v6).collect();
        assert_eq!(v4_set.len(), 40);
        assert_eq!(v6_set.len(), 40);
        for addresses in first.values() {
            assert!(v4("100.64.0.0/24").contains(addresses.v4));
            assert!(v6("fd00:1::/64").contains(addresses.v6));
        }
    }

    #[test]
    fn golden_first_candidate() {
        // Pins the hash construction: changing it moves every device.
        let map = allocate(v4("100.64.0.0/10"), v6("fd00::/48"), &[node(1)]).unwrap();
        let addresses = map[&node(1)];
        assert_eq!(addresses.v4, GOLDEN_V4);
        assert_eq!(addresses.v6, GOLDEN_V6);
    }

    const GOLDEN_V4: Ipv4Addr = Ipv4Addr::new(100, 98, 226, 39);
    const GOLDEN_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0x6b1f, 0x16b5, 0x1e56, 0x585, 0x9074);

    #[test]
    fn small_range_reserves_and_fills() {
        // A /29 has 8 addresses: network, resolver and broadcast leave 5.
        let range = v4("10.0.0.0/29");
        let map = allocate(range, v6("fd00::/120"), &nodes(5)).unwrap();
        let got: BTreeSet<_> = map.values().map(|a| a.v4).collect();
        let expected: BTreeSet<_> = (2..=6).map(|host| Ipv4Addr::new(10, 0, 0, host)).collect();
        assert_eq!(got, expected);
        assert_eq!(
            allocate(range, v6("fd00::/120"), &nodes(6)),
            Err(AllocationError::RangeFull { family: Family::V4 })
        );
        // IPv6 /126: four addresses, two reserved.
        assert_eq!(
            allocate(v4("10.0.0.0/24"), v6("fd00::/126"), &nodes(3)),
            Err(AllocationError::RangeFull { family: Family::V6 })
        );
    }

    #[test]
    fn too_small_ranges() {
        assert!(allocate(v4("10.0.0.0/30"), v6("fd00::/64"), &nodes(1)).is_ok());
        assert_eq!(
            allocate(v4("10.0.0.0/31"), v6("fd00::/64"), &[]),
            Err(AllocationError::RangeTooSmall {
                family: Family::V4,
                prefix_len: 31,
                max: 30
            })
        );
        assert_eq!(
            allocate(v4("10.0.0.0/24"), v6("fd00::/127"), &[]),
            Err(AllocationError::RangeTooSmall {
                family: Family::V6,
                prefix_len: 127,
                max: 126
            })
        );
        assert!(resolver_address(IpPrefix::V4(v4("10.0.0.1/32"))).is_err());
    }

    #[test]
    fn whole_address_space_works() {
        let map = allocate(v4("0.0.0.0/0"), v6("::/0"), &nodes(3)).unwrap();
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn duplicate_member_errors() {
        assert_eq!(
            allocate(
                v4("10.0.0.0/24"),
                v6("fd00::/64"),
                &[node(1), node(2), node(1)]
            ),
            Err(AllocationError::DuplicateMember(node(1)))
        );
    }

    #[test]
    fn resolver_is_first_host() {
        assert_eq!(
            resolver_address(IpPrefix::V4(v4("100.64.0.0/10"))),
            Ok(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)))
        );
        assert_eq!(
            resolver_address(IpPrefix::V6(v6("fd00:1::/48"))),
            Ok(IpAddr::V6("fd00:1::1".parse().unwrap()))
        );
    }

    proptest! {
        #[test]
        fn adding_members_never_moves_existing_ones(
            ids in proptest::collection::btree_set(any::<[u8; 32]>(), 1..30),
            split in 0_usize..30,
        ) {
            let members: Vec<NodeId> = ids.into_iter().map(NodeId::from_bytes).collect();
            let split = split.min(members.len());
            // A /27 forces collisions, so probing is exercised.
            let (range4, range6) = (v4("10.9.8.0/27"), v6("fd00::/123"));
            let before = allocate(range4, range6, &members[..split]).unwrap();
            let after = allocate(range4, range6, &members).unwrap();
            for (node, addresses) in &before {
                prop_assert_eq!(after.get(node), Some(addresses));
            }
            for addresses in after.values() {
                let host = addresses.v4.octets()[3] & 31;
                prop_assert!((2..=30).contains(&host));
                let host6 = u128::from(addresses.v6) & 31;
                prop_assert!(host6 >= 2);
            }
        }
    }
}
