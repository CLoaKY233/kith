//! Compiled filters: hand-written cases, and agreement with a naive reference evaluator.

use alloc::collections::BTreeSet;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

extern crate std;

use crate::{
    Addresses, Member, Members, PacketProtocol, Policy, PortRange, PortSet, Protocol, Rule,
    Selector, TagName, Target, compile,
};
use alloc::vec;
use alloc::vec::Vec;
use kith_types::NodeId;
use proptest::prelude::*;

const NODES: u8 = 5;
const TAGS: [&str; 3] = ["srv", "dev", "ops"];

fn node(index: u8) -> NodeId {
    NodeId::from_bytes([index; 32])
}

fn tag(name: &str) -> TagName {
    TagName::new(name).unwrap()
}

fn addresses(index: u8) -> Addresses {
    Addresses {
        v4: Ipv4Addr::new(100, 64, 0, index + 2),
        v6: Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, u16::from(index) + 2),
    }
}

fn v4(index: u8) -> IpAddr {
    IpAddr::V4(addresses(index).v4)
}

fn members(tags: &[&[&str]]) -> Members {
    tags.iter()
        .zip(0..)
        .map(|(names, index)| {
            let member = Member {
                tags: names.iter().map(|name| tag(name)).collect(),
                addresses: addresses(index),
            };
            (node(index), member)
        })
        .collect()
}

fn rule(sources: Vec<Selector>, destinations: Vec<Target>) -> Rule {
    Rule::new(sources, destinations).unwrap()
}

fn policy(rules: Vec<Rule>) -> Policy {
    Policy::new(rules).unwrap()
}

#[test]
fn default_deny() {
    let members = members(&[&[], &[]]);
    let filter = compile(&Policy::deny_all(), &members, node(0));
    assert_eq!(filter.entries(), []);
    assert!(!filter.allows(v4(1), PacketProtocol::Tcp, Some(22)));
    assert!(!filter.allows(v4(1), PacketProtocol::Icmp, None));
    // Self-traffic is always allowed, on both families.
    assert!(filter.allows(v4(0), PacketProtocol::Udp, Some(53)));
    assert!(filter.allows(IpAddr::V6(addresses(0).v6), PacketProtocol::Other(47), None));
}

#[test]
fn allow_all_reaches_members_only() {
    let members = members(&[&[], &[], &[]]);
    let filter = compile(&Policy::default_allow_all(), &members, node(0));
    assert_eq!(filter.entries().len(), 1);
    assert_eq!(filter.entries()[0].sources().len(), 6);
    for peer in 1..3 {
        assert!(filter.allows(v4(peer), PacketProtocol::Other(50), None));
        assert!(filter.allows(IpAddr::V6(addresses(peer).v6), PacketProtocol::Tcp, Some(1)));
    }
    // Unknown sources are denied.
    assert!(!filter.allows(
        IpAddr::V4(Ipv4Addr::new(100, 64, 0, 99)),
        PacketProtocol::Tcp,
        Some(1)
    ));
    assert!(!filter.allows(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        PacketProtocol::Tcp,
        Some(1)
    ));
}

#[test]
fn non_member_local_gets_nothing() {
    let members = members(&[&[], &[]]);
    let filter = compile(&Policy::default_allow_all(), &members, node(9));
    assert!(!filter.allows(v4(0), PacketProtocol::Tcp, Some(1)));
    assert_eq!(filter.local_addresses(), None);
}

#[test]
fn tags_devices_protocols_and_ports() {
    use PacketProtocol::{Icmp, Other, Tcp, Udp};

    // 0: srv (local), 1: dev, 2: ops, 3: dev+ops, 4: untagged.
    let members = members(&[&["srv"], &["dev"], &["ops"], &["dev", "ops"], &[]]);
    let ssh = Target::new(
        Selector::Tag(tag("srv")),
        Protocol::Tcp,
        PortSet::single(22),
    )
    .unwrap();
    let dns = Target::new(
        Selector::Device(node(0)),
        Protocol::Udp,
        PortSet::ranges(vec![
            PortRange::single(53),
            PortRange::new(5353, 5354).unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    let ping = Target::new(Selector::Any, Protocol::Icmp, PortSet::All).unwrap();
    let elsewhere = Target::all_traffic(Selector::Tag(tag("ops")));
    let policy = policy(vec![
        rule(vec![Selector::Tag(tag("dev"))], vec![ssh, elsewhere]),
        rule(vec![Selector::Device(node(2))], vec![dns]),
        rule(vec![Selector::Any], vec![ping]),
    ]);
    let filter = compile(&policy, &members, node(0));

    // dev members reach ssh, and nothing else but ICMP.
    for peer in [1, 3] {
        assert!(filter.allows(v4(peer), Tcp, Some(22)));
        assert!(!filter.allows(v4(peer), Tcp, Some(23)));
        assert!(!filter.allows(v4(peer), Udp, Some(22)));
        assert!(!filter.allows(v4(peer), Tcp, None));
    }
    // The ops device reaches DNS by device selector.
    assert!(filter.allows(v4(2), Udp, Some(53)));
    assert!(filter.allows(v4(2), Udp, Some(5354)));
    assert!(!filter.allows(v4(2), Udp, Some(5355)));
    assert!(!filter.allows(v4(2), Tcp, Some(53)));
    assert!(!filter.allows(v4(2), Tcp, Some(22)));
    // Everyone pings; nothing else for the untagged device.
    for peer in 1..5 {
        assert!(filter.allows(v4(peer), Icmp, None));
    }
    assert!(!filter.allows(v4(4), Tcp, Some(22)));
    assert!(!filter.allows(v4(4), Other(1), None));

    // From an ops device's view, dev members may send anything.
    let ops = compile(&policy, &members, node(2));
    assert!(ops.allows(v4(1), Other(47), None));
    assert!(ops.allows(v4(3), Tcp, Some(9999)));
    assert!(!ops.allows(v4(0), Tcp, Some(22)));
}

#[test]
fn merges_and_is_deterministic() {
    let members = members(&[&["srv"], &["dev"], &["dev"]]);
    let ssh = || Target::new(Selector::Any, Protocol::Tcp, PortSet::single(22)).unwrap();
    let rules = vec![
        rule(vec![Selector::Tag(tag("dev"))], vec![ssh()]),
        rule(
            vec![Selector::Device(node(1)), Selector::Device(node(1))],
            vec![ssh(), ssh()],
        ),
        rule(
            vec![Selector::Device(node(2))],
            vec![Target::all_traffic(Selector::Any)],
        ),
    ];
    let mut reversed = rules.clone();
    reversed.reverse();
    let forward = compile(&policy(rules), &members, node(0));
    assert_eq!(forward, compile(&policy(reversed), &members, node(0)));
    // Node 1 only via ssh; node 2 needs only the unrestricted entry.
    assert_eq!(forward.entries().len(), 2);
    for entry in forward.entries() {
        assert_eq!(entry.sources().len(), 2);
        let sorted: Vec<_> = entry
            .sources()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(entry.sources(), sorted.as_slice());
        assert!(
            entry
                .sources()
                .iter()
                .all(|prefix| matches!(prefix.prefix_len(), 32 | 128))
        );
    }
}

fn reference(
    policy: &Policy,
    members: &Members,
    local: NodeId,
    source: IpAddr,
    protocol: PacketProtocol,
    port: Option<u16>,
) -> bool {
    let Some(local_member) = members.get(&local) else {
        return false;
    };
    let has_address = |member: &Member| match source {
        IpAddr::V4(address) => member.addresses.v4 == address,
        IpAddr::V6(address) => member.addresses.v6 == address,
    };
    if has_address(local_member) {
        return true;
    }
    let selects = |selector: &Selector, node: &NodeId, member: &Member| match selector {
        Selector::Any => true,
        Selector::Device(device) => device == node,
        Selector::Tag(tag) => member.tags.contains(tag),
    };
    let protocol_ok = |wanted: Protocol| match wanted {
        Protocol::Any => true,
        Protocol::Tcp => protocol == PacketProtocol::Tcp,
        Protocol::Udp => protocol == PacketProtocol::Udp,
        Protocol::Icmp => protocol == PacketProtocol::Icmp,
    };
    let port_ok = |ports: &PortSet| match ports {
        PortSet::All => true,
        PortSet::Only(ranges) => port.is_some_and(|port| {
            ranges
                .as_slice()
                .iter()
                .any(|range| range.start() <= port && port <= range.end())
        }),
    };
    policy.rules().iter().any(|rule| {
        let source_ok = members.iter().any(|(node, member)| {
            has_address(member) && rule.sources().iter().any(|s| selects(s, node, member))
        });
        source_ok
            && rule.destinations().iter().any(|target| {
                selects(target.selector(), &local, local_member)
                    && protocol_ok(target.protocol())
                    && port_ok(target.ports())
            })
    })
}

fn selector() -> impl Strategy<Value = Selector> {
    prop_oneof![
        Just(Selector::Any),
        (0..NODES + 1).prop_map(|index| Selector::Device(node(index))),
        (0..TAGS.len()).prop_map(|index| Selector::Tag(tag(TAGS[index]))),
    ]
}

fn port_set() -> impl Strategy<Value = PortSet> {
    prop_oneof![
        Just(PortSet::All),
        proptest::collection::vec((0_u16..30, 0_u16..6), 1..5).prop_map(|ranges| {
            let ranges = ranges
                .into_iter()
                .map(|(start, width)| PortRange::new(start, start + width).unwrap())
                .collect();
            PortSet::ranges(ranges).unwrap()
        }),
    ]
}

fn target() -> impl Strategy<Value = Target> {
    let protocol = prop_oneof![
        Just(Protocol::Any),
        Just(Protocol::Tcp),
        Just(Protocol::Udp),
        Just(Protocol::Icmp)
    ];
    (selector(), protocol, port_set()).prop_map(|(selector, protocol, ports)| {
        let ports = if protocol == Protocol::Icmp {
            PortSet::All
        } else {
            ports
        };
        Target::new(selector, protocol, ports).unwrap()
    })
}

fn policy_strategy() -> impl Strategy<Value = Policy> {
    let rule = (
        proptest::collection::vec(selector(), 1..4),
        proptest::collection::vec(target(), 1..4),
    )
        .prop_map(|(sources, destinations)| Rule::new(sources, destinations).unwrap());
    proptest::collection::vec(rule, 0..6).prop_map(|rules| Policy::new(rules).unwrap())
}

/// Members drawn from a small pool; some addresses are shared to stress merging.
fn members_strategy() -> impl Strategy<Value = Members> {
    proptest::collection::btree_map(
        0..NODES,
        (
            proptest::collection::btree_set(0..TAGS.len(), 0..3),
            0_u8..NODES,
        ),
        0..=usize::from(NODES),
    )
    .prop_map(|chosen| {
        chosen
            .into_iter()
            .map(|(index, (tags, address))| {
                let member = Member {
                    tags: tags
                        .into_iter()
                        .map(|tag_index| tag(TAGS[tag_index]))
                        .collect(),
                    addresses: addresses(address),
                };
                (node(index), member)
            })
            .collect()
    })
}

fn packet() -> impl Strategy<Value = (IpAddr, PacketProtocol, Option<u16>)> {
    let source = prop_oneof![
        (0..NODES + 1).prop_map(|index| IpAddr::V4(addresses(index).v4)),
        (0..NODES + 1).prop_map(|index| IpAddr::V6(addresses(index).v6)),
    ];
    let protocol = prop_oneof![
        Just(PacketProtocol::Tcp),
        Just(PacketProtocol::Udp),
        Just(PacketProtocol::Icmp),
        any::<u8>().prop_map(PacketProtocol::Other),
    ];
    (source, protocol, proptest::option::of(0_u16..40))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn compiled_filter_agrees_with_reference(
        policy in policy_strategy(),
        members in members_strategy(),
        local in 0..NODES + 1,
        packets in proptest::collection::vec(packet(), 1..32),
    ) {
        let local = node(local);
        let filter = compile(&policy, &members, local);
        for (source, protocol, port) in packets {
            prop_assert_eq!(
                filter.allows(source, protocol, port),
                reference(&policy, &members, local, source, protocol, port),
                "source {} protocol {:?} port {:?}", source, protocol, port
            );
        }
    }
}
