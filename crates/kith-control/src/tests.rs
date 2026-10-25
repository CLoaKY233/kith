//! Tests of map derivation, diffs and the multi-network manager, on real ledgers.

extern crate std;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::IpAddr;

use kith_crypto::SigningKey;
use kith_ledger::{Ledger, NetworkParams, OpBody, SignedOp};
use kith_policy::{
    PacketProtocol, Policy, PortSet, Protocol, Rule, Selector, TagName, Target, allocate,
};
use kith_types::{NodeId, OpId, Timestamp};
use proptest::prelude::*;

use crate::{NetMap, Networks, Warning, build_netmap, records};

const NOW: Timestamp = Timestamp::from_millis(1_000);

fn key(seed: u8) -> SigningKey {
    SigningKey::from_seed(&[seed; 32])
}

fn id(seed: u8) -> NodeId {
    key(seed).node_id()
}

fn params(ipv4: &str, ipv6: &str) -> NetworkParams {
    NetworkParams {
        ipv4: ipv4.parse().unwrap(),
        ipv6: ipv6.parse().unwrap(),
    }
}

/// A network founded by key 1 ("laptop"), with key 2 as a second admin.
fn network() -> Ledger {
    let params = params("100.64.0.0/10", "fd7a:115c:a1e0::/48");
    Ledger::create(&key(1), "home", "laptop", params, &[id(2)], 1, NOW).unwrap()
}

fn sign(ledger: &Ledger, author: u8, body: OpBody) -> SignedOp {
    ledger
        .draft(id(author), body, NOW)
        .sign(&key(author))
        .unwrap()
}

fn act(ledger: &mut Ledger, author: u8, body: OpBody) {
    let op = sign(ledger, author, body);
    ledger.insert(op).unwrap();
}

fn add_device(seed: u8, name: &str, tags: &[&str]) -> OpBody {
    OpBody::AddDevice {
        device: id(seed),
        name: String::from(name),
        tags: tags.iter().map(|tag| String::from(*tag)).collect(),
    }
}

fn set_record(key: &str, value: Vec<u8>) -> OpBody {
    OpBody::SetRecord {
        key: String::from(key),
        value: Some(value),
    }
}

fn tag(name: &str) -> Selector {
    Selector::Tag(TagName::new(name).unwrap())
}

/// Devices tagged `phone` may reach devices tagged `server` on TCP 22, nothing else.
fn ssh_policy() -> Vec<u8> {
    let rule = Rule::new(
        vec![tag("phone")],
        vec![Target::new(tag("server"), Protocol::Tcp, PortSet::single(22)).unwrap()],
    )
    .unwrap();
    Policy::new(vec![rule]).unwrap().to_bytes().unwrap()
}

fn v4(map: &NetMap, seed: u8) -> IpAddr {
    IpAddr::V4(map.peers()[&id(seed)].addresses.v4)
}

#[test]
fn netmap_follows_members_admission_order_and_policy() {
    let mut ledger = network();
    act(&mut ledger, 1, add_device(3, "server", &["server"]));
    act(
        &mut ledger,
        2,
        add_device(4, "phone", &["Not_A_Tag", "phone"]),
    );
    act(&mut ledger, 1, set_record(records::POLICY, ssh_policy()));
    let route = alloc::format!("{}192.168.1.0/24", records::ROUTE_PREFIX);
    act(
        &mut ledger,
        1,
        set_record(&route, id(3).to_bytes().to_vec()),
    );

    let map = build_netmap(ledger.view(), id(3)).unwrap();
    assert!(map.is_member());
    assert_eq!(map.name(), "home");
    assert_eq!(
        map.peers().keys().copied().collect::<Vec<_>>(),
        {
            let mut expected = vec![id(1), id(4)];
            expected.sort();
            expected
        },
        "every member but the local device"
    );
    assert_eq!(map.peers()[&id(4)].name, "phone");
    assert_eq!(
        map.warnings(),
        [Warning::InvalidTag {
            device: id(4),
            tag: String::from("Not_A_Tag")
        }]
    );

    // Addresses follow admission order (genesis, then each add), not node-id order.
    let expected = allocate(map.ipv4_range(), map.ipv6_range(), &[id(1), id(3), id(4)]).unwrap();
    assert_eq!(map.local_addresses(), Some(expected[&id(3)]));
    assert_eq!(map.peers()[&id(1)].addresses, expected[&id(1)]);
    assert_eq!(map.peers()[&id(4)].addresses, expected[&id(4)]);
    assert_eq!(map.resolver()[0], "100.64.0.1".parse::<IpAddr>().unwrap());
    assert_eq!(
        map.routes().iter().collect::<Vec<_>>(),
        [(&"192.168.1.0/24".parse().unwrap(), &id(3))]
    );

    let filter = map.filter();
    let phone = v4(&map, 4);
    let ssh = (PacketProtocol::Tcp, Some(22));
    assert!(filter.allows(phone, ssh.0, ssh.1));
    assert!(!filter.allows(phone, PacketProtocol::Tcp, Some(23)));
    assert!(
        !filter.allows(v4(&map, 1), ssh.0, ssh.1),
        "laptop is not a phone"
    );
    let own = IpAddr::V4(expected[&id(3)].v4);
    assert!(filter.allows(own, PacketProtocol::Udp, Some(53)));

    // The phone is not a server, so nothing may reach it.
    let phone_map = build_netmap(ledger.view(), id(4)).unwrap();
    assert!(!phone_map.filter().allows(v4(&phone_map, 3), ssh.0, ssh.1));
}

#[test]
fn removed_local_device_gets_an_empty_map_not_an_error() {
    let mut ledger = network();
    act(&mut ledger, 1, add_device(3, "server", &[]));
    act(&mut ledger, 1, OpBody::RemoveDevice { device: id(3) });

    let map = build_netmap(ledger.view(), id(3)).unwrap();
    assert!(!map.is_member());
    assert!(map.peers().is_empty());
    assert_eq!(map.local_addresses(), None);
    let laptop = allocate(map.ipv4_range(), map.ipv6_range(), &[id(1)]).unwrap()[&id(1)];
    assert!(
        !map.filter()
            .allows(IpAddr::V4(laptop.v4), PacketProtocol::Icmp, None)
    );
}

#[test]
fn undecodable_policy_denies_peers_keeps_self_and_warns() {
    let mut ledger = network();
    act(&mut ledger, 1, add_device(3, "server", &[]));
    act(
        &mut ledger,
        1,
        set_record(records::POLICY, b"not a policy".to_vec()),
    );

    let map = build_netmap(ledger.view(), id(1)).unwrap();
    assert!(matches!(map.warnings(), [Warning::InvalidPolicy(_)]));
    assert!(
        !map.filter()
            .allows(v4(&map, 3), PacketProtocol::Tcp, Some(22))
    );
    let own = IpAddr::V4(map.local_addresses().unwrap().v4);
    assert!(map.filter().allows(own, PacketProtocol::Tcp, Some(22)));
}

#[test]
fn overlapping_mesh_ranges_are_reported_as_conflicts() {
    let found = |seed: u8, ipv4: &str, ipv6: &str| {
        Ledger::create(&key(seed), "net", "dev", params(ipv4, ipv6), &[], 1, NOW).unwrap()
    };
    let wide = found(1, "100.64.0.0/10", "fd00:1::/48");
    let inside = found(2, "100.100.0.0/16", "fd00:2::/48");
    let apart = found(3, "10.0.0.0/8", "fd00:3::/48");

    let mut networks = Networks::new();
    for ledger in [&wide, &inside, &apart] {
        let diff = networks.update(ledger.view(), id(1)).unwrap();
        assert!(diff.filter_changed, "a new network is a full change");
    }
    let conflicts = networks.conflicts();
    assert_eq!(conflicts.len(), 1);
    let conflict = conflicts[0];
    let mut pair = [conflict.first, conflict.second];
    pair.sort();
    let mut expected = [wide.network_id(), inside.network_id()];
    expected.sort();
    assert_eq!(pair, expected);
    assert!(conflict.first < conflict.second);

    networks.remove(&inside.network_id());
    assert_eq!(networks.conflicts(), []);
}

/// Deterministic Fisher–Yates shuffle driven by xorshift, as in the ledger tests.
fn shuffle<T>(items: &mut [T], mut seed: u64) {
    seed |= 1;
    for index in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let other = usize::try_from(seed % (u64::try_from(index).unwrap() + 1)).unwrap();
        items.swap(index, other);
    }
}

/// One random admin action. Devices are keys 1 and 10 to 15.
fn random_body(action: u8, target: u8) -> OpBody {
    let device = if target == 0 { 1 } else { 9 + target };
    let tags: &[&[&str]] = &[&[], &["phone"], &["server"], &["phone", "server"]];
    match action {
        0..=2 => add_device(device, "dev", tags[usize::from(action)]),
        3 | 4 => OpBody::RemoveDevice { device: id(device) },
        5 => set_record(records::POLICY, ssh_policy()),
        6 => set_record(records::POLICY, vec![target]),
        7 => OpBody::SetRecord {
            key: String::from(records::POLICY),
            value: None,
        },
        _ => {
            let route = alloc::format!("{}10.1.0.0/16", records::ROUTE_PREFIX);
            set_record(&route, id(device).to_bytes().to_vec())
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Two admins act concurrently and gossip; any insertion order of the resulting ops
    /// yields the same map for every device.
    #[test]
    fn netmaps_are_identical_for_any_op_order(
        steps in proptest::collection::vec((0_usize..3, 0_u8..9, 0_u8..7), 1..30),
        seeds in proptest::collection::vec(any::<u64>(), 2),
    ) {
        let base = network();
        let mut replicas = [base.clone(), base.clone()];
        for (writer, action, target) in steps {
            if writer == 2 {
                let ops: Vec<SignedOp> = replicas[0]
                    .op_ids()
                    .filter_map(|op| replicas[0].get(op).cloned())
                    .collect();
                replicas[1].insert_batch(ops);
                continue;
            }
            let author = u8::try_from(writer).unwrap() + 1;
            let op = sign(&replicas[writer], author, random_body(action, target));
            replicas[writer].insert(op).unwrap();
        }
        let all: BTreeMap<OpId, SignedOp> = replicas
            .iter()
            .flat_map(|replica| replica.op_ids().map(|op| (*op, replica.get(op).unwrap().clone())))
            .collect();

        let locals = [1, 2, 10, 11, 12];
        let mut reference: Option<Vec<NetMap>> = None;
        for seed in seeds {
            let mut ops: Vec<SignedOp> = all.values().cloned().collect();
            shuffle(&mut ops, seed);
            let mut ledger = base.clone();
            for op in ops {
                ledger.insert(op).unwrap();
            }
            prop_assert_eq!(ledger.orphan_count(), 0);
            let maps: Vec<NetMap> = locals
                .iter()
                .map(|seed| build_netmap(ledger.view(), id(*seed)).unwrap())
                .collect();
            match &reference {
                None => reference = Some(maps),
                Some(expected) => prop_assert_eq!(expected, &maps),
            }
        }
    }

    /// Applying the diff between any two maps of a network to the old peers gives the new
    /// peers, and the flags say exactly what else moved.
    #[test]
    fn applying_a_diff_reproduces_the_new_map(
        steps in proptest::collection::vec((0_u8..9, 0_u8..7), 1..30),
        pairs in proptest::collection::vec((any::<prop::sample::Index>(), any::<prop::sample::Index>()), 8),
    ) {
        let mut ledger = network();
        let mut maps = vec![build_netmap(ledger.view(), id(10)).unwrap()];
        for (action, target) in steps {
            act(&mut ledger, 2, random_body(action, target));
            maps.push(build_netmap(ledger.view(), id(10)).unwrap());
        }
        for (old, new) in pairs {
            let (old, new) = (old.get(&maps), new.get(&maps));
            let diff = NetMap::diff(old, new);
            let mut peers = old.peers().clone();
            diff.apply_to_peers(&mut peers);
            prop_assert_eq!(&peers, new.peers());
            prop_assert!(diff.peers_changed.keys().all(|node| old.peers().contains_key(node)));
            prop_assert_eq!(diff.filter_changed, old.filter() != new.filter());
            prop_assert_eq!(diff.local_addresses_changed, old.local_addresses() != new.local_addresses());
            prop_assert_eq!(diff.routes_changed, old.routes() != new.routes());
            prop_assert_eq!(diff.is_empty(), old.peers() == new.peers()
                && old.filter() == new.filter()
                && old.local_addresses() == new.local_addresses()
                && old.routes() == new.routes());
        }
    }
}
