//! End-to-end: two engines over loopback UDP, with in-memory TUNs.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use kith_crypto::SigningKey;
use kith_policy::{Addresses, Member, Members, Policy, compile};
use kith_router::MeshRanges;
use kith_transport_udp::UdpLink;
use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NetworkId, NodeId};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::engine::Engine;
use crate::memory_tun::{MemoryTun, memory_tun_with_mtu};
use crate::{
    Dataplane, DataplaneHandle, DropCounters, Endpoint, Event, ExternalInterface, ExternalMember,
    ExternalPeer, PeerSpec,
};

const NETWORK: NetworkId = NetworkId::from_bytes([7; 32]);
const PATIENCE: Duration = Duration::from_secs(5);

struct Node {
    id: NodeId,
    addresses: Addresses,
    udp: SocketAddr,
    handle: DataplaneHandle,
    events: mpsc::Receiver<Event>,
    tun: MemoryTun,
}

fn spawn_node(index: u8) -> Node {
    spawn_node_with_mtu(index, crate::memory_tun::DEFAULT_MTU)
}

fn spawn_node_with_mtu(index: u8, mtu: u16) -> Node {
    let key = SigningKey::from_seed(&[index; 32]);
    let link = UdpLink::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let udp = link.local_addr();
    let (device, tun) = memory_tun_with_mtu(mtu);
    let (engine, handle, events) = Engine::new(&key, device, link).unwrap();
    tokio::spawn(engine.run());
    Node {
        id: key.node_id(),
        addresses: Addresses {
            v4: Ipv4Addr::new(100, 64, 0, index),
            v6: Ipv6Addr::new(0xfd7a, 0, 0, 0, 0, 0, 0, u16::from(index)),
        },
        udp,
        handle,
        events,
        tun,
    }
}

/// Makes `peer` reachable from `node` at `endpoint`, with `node` admitting by `policy`.
async fn connect(node: &Node, peer: &Node, endpoint: Endpoint, policy: &Policy) {
    let host = IpPrefix::V4(Ipv4Prefix::new(peer.addresses.v4, 32).unwrap());
    node.handle
        .set_peers(vec![PeerSpec {
            network: NETWORK,
            node: peer.id,
            allowed_ips: vec![host],
            endpoint: Some(endpoint),
            persistent_keepalive: None,
        }])
        .await
        .unwrap();
    let mesh = MeshRanges {
        v4: Ipv4Prefix::new(Ipv4Addr::new(100, 64, 0, 0), 16).unwrap(),
        v6: Ipv6Prefix::new(Ipv6Addr::new(0xfd7a, 0, 0, 0, 0, 0, 0, 0), 48).unwrap(),
    };
    node.handle
        .set_routes(NETWORK, mesh, vec![(host, peer.id)])
        .await
        .unwrap();
    node.handle
        .set_filter(NETWORK, compile(policy, &members(&[node, peer]), node.id))
        .await
        .unwrap();
}

fn members(nodes: &[&Node]) -> Members {
    let mut members = Members::new();
    for node in nodes {
        let member = Member {
            tags: BTreeSet::new(),
            addresses: node.addresses,
        };
        members.insert(node.id, member);
    }
    members
}

/// An IPv4 UDP packet with a valid header checksum.
fn udp_packet(source: &Node, destination: &Node, payload: &[u8]) -> Vec<u8> {
    udp_between(source.addresses.v4, destination.addresses.v4, payload)
}

fn udp_between(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
    let total = u16::try_from(28 + payload.len()).unwrap();
    let mut packet = vec![0x45, 0, 0, 0, 0, 0, 0x40, 0, 64, 17, 0, 0];
    packet[2..4].copy_from_slice(&total.to_be_bytes());
    packet.extend_from_slice(&source.octets());
    packet.extend_from_slice(&destination.octets());
    let sum: u32 = packet
        .chunks(2)
        .map(|word| u32::from(u16::from_be_bytes([word[0], word[1]])))
        .sum();
    let folded = (sum & 0xffff) + (sum >> 16);
    packet[10..12].copy_from_slice(&(!u16::try_from(folded).unwrap()).to_be_bytes());
    packet.extend_from_slice(&[0x30, 0x39, 0x30, 0x39]);
    packet.extend_from_slice(&(total - 20).to_be_bytes());
    packet.extend_from_slice(&[0, 0]);
    packet.extend_from_slice(payload);
    packet
}

async fn next_packet(node: &mut Node) -> Vec<u8> {
    timeout(PATIENCE, node.tun.receive())
        .await
        .unwrap()
        .unwrap()
}

async fn next_event(node: &mut Node) -> Event {
    timeout(PATIENCE, node.events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn packets_cross_loopback_intact_and_denied_ones_drop() {
    let mut a = spawn_node(1);
    let mut b = spawn_node(2);
    connect(
        &a,
        &b,
        Endpoint::Direct(b.udp),
        &Policy::default_allow_all(),
    )
    .await;
    connect(&b, &a, Endpoint::Direct(a.udp), &Policy::deny_all()).await;

    a.tun.inject(udp_packet(&a, &b, b"denied")).unwrap();
    timeout(PATIENCE, async {
        while b.handle.stats().await.unwrap().drops.policy == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(b.handle.stats().await.unwrap().drops.policy, 1);

    let allow = compile(&Policy::default_allow_all(), &members(&[&b, &a]), b.id);
    b.handle.set_filter(NETWORK, allow).await.unwrap();
    let request = udp_packet(&a, &b, b"allowed");
    a.tun.inject(request.clone()).unwrap();
    assert_eq!(next_packet(&mut b).await, request);

    let reply = udp_packet(&b, &a, b"reply");
    b.tun.inject(reply.clone()).unwrap();
    assert_eq!(next_packet(&mut a).await, reply);
}

#[tokio::test]
async fn exit_traffic_and_its_replies_cross_the_exit_peer() {
    let mut a = spawn_node(1);
    let mut b = spawn_node(2);
    let allow = Policy::default_allow_all();
    connect(&a, &b, Endpoint::Direct(b.udp), &allow).await;
    connect(&b, &a, Endpoint::Direct(a.udp), &allow).await;
    a.handle.set_exit(Some((NETWORK, b.id))).await.unwrap();
    a.handle
        .set_addresses(NETWORK, vec![(a.addresses.v4.into(), 16)])
        .await
        .unwrap();

    let internet = Ipv4Addr::new(1, 1, 1, 1);
    a.tun
        .inject(udp_between(
            Ipv4Addr::new(192, 168, 1, 5),
            internet,
            b"stale",
        ))
        .unwrap();
    timeout(PATIENCE, async {
        while a.handle.stats().await.unwrap().drops.foreign_source == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        b.tun.try_receive().is_none(),
        "foreign-source traffic must not reach the exit"
    );
    let request = udp_between(a.addresses.v4, internet, b"out");
    a.tun.inject(request.clone()).unwrap();
    assert_eq!(next_packet(&mut b).await, request);

    let reply = udp_between(internet, a.addresses.v4, b"back");
    b.tun.inject(reply.clone()).unwrap();
    assert_eq!(next_packet(&mut a).await, reply);

    a.handle.set_exit(None).await.unwrap();
    b.tun
        .inject(udp_between(internet, a.addresses.v4, b"late"))
        .unwrap();
    let drops = timeout(PATIENCE, async {
        loop {
            let drops = a.handle.stats().await.unwrap().drops;
            if drops.after_exit > 0 {
                break drops;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        drops.source_not_allowed, 0,
        "late exit replies are not a spoofing alarm"
    );
    assert!(a.tun.try_receive().is_none());
}

#[tokio::test]
async fn stats_track_a_peer_until_its_network_is_removed() {
    let a = spawn_node(7);
    let mut b = spawn_node(8);
    let allow = Policy::default_allow_all();
    connect(&a, &b, Endpoint::Direct(b.udp), &allow).await;
    connect(&b, &a, Endpoint::Direct(a.udp), &allow).await;
    let request = udp_packet(&a, &b, b"counted");
    a.tun.inject(request.clone()).unwrap();
    assert_eq!(next_packet(&mut b).await, request);

    let stats = b.handle.stats().await.unwrap();
    let [peer] = stats.peers.as_slice() else {
        panic!("expected one peer, got {:?}", stats.peers);
    };
    assert_eq!(
        (peer.network, peer.node, peer.endpoint),
        (NETWORK, a.id, Some(Endpoint::Direct(a.udp)))
    );
    assert!(peer.last_handshake.is_some());
    assert!(peer.rx_bytes > u64::try_from(request.len()).unwrap() && peer.tx_bytes > 0);

    b.handle.remove_network(NETWORK).await.unwrap();
    a.tun.inject(udp_packet(&a, &b, b"orphaned")).unwrap();
    b.tun.inject(udp_packet(&b, &a, b"unroutable")).unwrap();
    let drops = timeout(PATIENCE, async {
        loop {
            let stats = b.handle.stats().await.unwrap();
            assert_eq!(stats.peers, []);
            if stats.drops.invalid > 0 && stats.drops.no_route > 0 {
                break stats.drops;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // Both sides handshake on connect, so a late handshake reply may add to `invalid`.
    let expected = DropCounters {
        invalid: drops.invalid,
        no_route: 1,
        ..DropCounters::default()
    };
    assert_eq!(drops, expected);
    assert!(b.tun.try_receive().is_none());
}

#[tokio::test]
async fn relayed_peers_exchange_datagrams_through_events() {
    let mut a = spawn_node(3);
    let mut b = spawn_node(4);
    connect(&a, &b, Endpoint::Relay, &Policy::default_allow_all()).await;
    connect(&b, &a, Endpoint::Relay, &Policy::default_allow_all()).await;

    let packet = udp_packet(&a, &b, b"via relay");
    a.tun.inject(packet.clone()).unwrap();
    let delivered = timeout(PATIENCE, async {
        loop {
            tokio::select! {
                Some(event) = a.events.recv() => if let Event::Relay { peer, datagram } = event {
                    assert_eq!(peer, b.id);
                    b.handle.accept_relayed(a.id, datagram).await.unwrap();
                },
                Some(event) = b.events.recv() => if let Event::Relay { peer, datagram } = event {
                    assert_eq!(peer, a.id);
                    a.handle.accept_relayed(b.id, datagram).await.unwrap();
                },
                Some(delivered) = b.tun.receive() => break delivered,
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(delivered, packet);
}

#[tokio::test]
async fn raw_sends_arrive_as_control_events() {
    let a = spawn_node(5);
    let mut b = spawn_node(6);
    let mut stun = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xa4, 0x42];
    stun.extend_from_slice(&[9; 12]);

    a.handle.send_raw(b.udp, stun.clone()).await.unwrap();
    assert_eq!(
        next_event(&mut b).await,
        Event::Control {
            source: a.udp,
            datagram: stun
        }
    );
}

#[tokio::test]
async fn external_interfaces_interoperate_beside_the_mesh() {
    let mut a = spawn_node(9);
    let mut b = spawn_node(10);
    let mesh_peer = spawn_node(11);
    connect(
        &a,
        &mesh_peer,
        Endpoint::Direct(mesh_peer.udp),
        &Policy::default_allow_all(),
    )
    .await;
    let (a_key, b_key, psk) = ([21; 32], [22; 32], Some([23; 32]));
    let public = |secret| kith_wg::StaticIdentity::from_secret_bytes(secret).public_key();
    let host = |node: &Node| IpPrefix::V4(Ipv4Prefix::new(node.addresses.v4, 32).unwrap());
    let interface = |key, peer_key, peer: &Node, own: &Node| ExternalInterface {
        name: "wg0".into(),
        private_key: key,
        addresses: vec![host(own)],
        peers: vec![ExternalPeer {
            public_key: public(peer_key),
            preshared_key: psk,
            endpoint: Some(peer.udp),
            allowed_ips: vec![host(peer)],
            persistent_keepalive: Some(25),
            member: None,
        }],
    };
    a.handle
        .set_external(interface(a_key, b_key, &b, &a))
        .await
        .unwrap();
    b.handle
        .set_external(interface(b_key, a_key, &a, &b))
        .await
        .unwrap();

    let request = udp_packet(&a, &b, b"external");
    a.tun.inject(request.clone()).unwrap();
    assert_eq!(next_packet(&mut b).await, request);
    let reply = udp_packet(&b, &a, b"external reply");
    b.tun.inject(reply.clone()).unwrap();
    assert_eq!(next_packet(&mut a).await, reply);

    let stats = b.handle.stats().await.unwrap();
    let [peer] = stats.external.as_slice() else {
        panic!("expected one external peer, got {:?}", stats.external);
    };
    assert_eq!(
        (peer.public_key, peer.endpoint),
        (public(a_key), Some(a.udp))
    );
    assert!(peer.last_handshake.is_some());
    assert_eq!(stats.drops, DropCounters::default());
}

/// Makes `phone` a plain WireGuard member behind `gateway`, guarded by `policy`.
async fn serve_phone(
    gateway: &Node,
    phone: &Node,
    policy: kith_policy::CompiledFilter,
    resolver: Ipv4Addr,
) {
    let (gateway_key, phone_key) = ([31; 32], [32; 32]);
    let public = |secret| kith_wg::StaticIdentity::from_secret_bytes(secret).public_key();
    let host = |v4: Ipv4Addr| IpPrefix::V4(Ipv4Prefix::new(v4, 32).unwrap());
    let everything = IpPrefix::V4(Ipv4Prefix::new(Ipv4Addr::UNSPECIFIED, 0).unwrap());
    phone
        .handle
        .set_external(ExternalInterface {
            name: "wg0".into(),
            private_key: phone_key,
            addresses: vec![host(phone.addresses.v4)],
            peers: vec![ExternalPeer {
                public_key: public(gateway_key),
                preshared_key: None,
                endpoint: Some(gateway.udp),
                allowed_ips: vec![everything],
                persistent_keepalive: Some(25),
                member: None,
            }],
        })
        .await
        .unwrap();
    gateway
        .handle
        .set_external(ExternalInterface {
            name: "kith-gateway".into(),
            private_key: gateway_key,
            addresses: Vec::new(),
            peers: vec![ExternalPeer {
                public_key: public(phone_key),
                preshared_key: None,
                endpoint: None,
                allowed_ips: vec![host(phone.addresses.v4)],
                persistent_keepalive: None,
                member: Some(ExternalMember {
                    network: NETWORK,
                    policy,
                    resolver: resolver.into(),
                }),
            }],
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_phone_reaches_its_gateway_only_in_its_own_network_and_always_its_resolver() {
    let mut gateway = spawn_node(12);
    let phone = spawn_node_with_mtu(13, 9000);
    let other = NetworkId::from_bytes([8; 32]);
    let resolver = Ipv4Addr::new(100, 64, 0, 1);
    let elsewhere = Ipv4Addr::new(100, 99, 0, 12);
    gateway
        .handle
        .set_addresses(
            NETWORK,
            vec![(gateway.addresses.v4.into(), 16), (resolver.into(), 32)],
        )
        .await
        .unwrap();
    gateway
        .handle
        .set_addresses(other, vec![(elsewhere.into(), 16)])
        .await
        .unwrap();
    let nobody = Policy::new(Vec::new()).unwrap();
    let everyone = Policy::default_allow_all();
    let both = members(&[&gateway, &phone]);
    gateway
        .handle
        .set_filter(NETWORK, compile(&everyone, &both, gateway.id))
        .await
        .unwrap();
    gateway
        .handle
        .set_filter(other, compile(&everyone, &both, gateway.id))
        .await
        .unwrap();

    serve_phone(
        &gateway,
        &phone,
        compile(&nobody, &both, phone.id),
        resolver,
    )
    .await;

    let to_gateway = udp_packet(&phone, &gateway, b"to its gateway");
    phone.tun.inject(to_gateway.clone()).unwrap();
    assert_eq!(next_packet(&mut gateway).await, to_gateway);
    phone.tun.inject(tcp_syn(&phone, &gateway, false)).unwrap();
    assert_eq!(
        announced_mss(&next_packet(&mut gateway).await),
        1160,
        "a phone runs no kith, so its gateway must lower what it announces"
    );

    let across = udp_between(phone.addresses.v4, elsewhere, b"into another network");
    phone.tun.inject(across).unwrap();
    let dns = udp_between(phone.addresses.v4, resolver, b"dns");
    phone.tun.inject(dns.clone()).unwrap();
    assert_eq!(
        next_packet(&mut gateway).await,
        dns,
        "the other network's address leaked through"
    );
    assert_eq!(gateway.handle.stats().await.unwrap().drops.policy, 1);

    let mut unasked = udp_packet(&gateway, &phone, b"nobody may reach the phone");
    unasked[20..22].copy_from_slice(&999_u16.to_be_bytes());
    gateway.tun.inject(unasked).unwrap();
    timeout(PATIENCE, async {
        while gateway.handle.stats().await.unwrap().drops.policy < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the phone's filter let in a packet its policy refuses");
}

/// A TCP SYN from `source` to `destination` announcing an MSS of 1460, as a host on an
/// Ethernet link sends, with valid checksums.
fn tcp_syn(source: &Node, destination: &Node, ack: bool) -> Vec<u8> {
    let flags = if ack { 0x12 } else { 0x02 };
    let mut packet = vec![0x45, 0, 0, 44, 0, 0, 0x40, 0, 64, 6, 0, 0];
    packet.extend_from_slice(&source.addresses.v4.octets());
    packet.extend_from_slice(&destination.addresses.v4.octets());
    packet.extend_from_slice(&[0x30, 0x39, 0x01, 0xbb, 0, 0, 0, 1, 0, 0, 0, 0, 0x60, flags]);
    packet.extend_from_slice(&[0xff, 0xff, 0, 0, 0, 0, 2, 4, 0x05, 0xb4]);
    kith_packet::Ipv4Packet::new_checked(&mut packet[..])
        .unwrap()
        .fill_checksum();
    let pseudo = kith_packet::IpPacket::new_checked(&packet[..])
        .unwrap()
        .pseudo_header();
    kith_packet::TcpPacket::new_checked(&mut packet[20..])
        .unwrap()
        .fill_checksum(&pseudo);
    packet
}

/// The MSS a SYN announces, after checking its TCP checksum still holds.
fn announced_mss(packet: &[u8]) -> u16 {
    let ip = kith_packet::IpPacket::new_checked(packet).unwrap();
    let tcp = kith_packet::TcpPacket::new_checked(ip.payload()).unwrap();
    assert!(
        tcp.verify_checksum(&ip.pseudo_header()),
        "the checksum broke"
    );
    u16::from_be_bytes([tcp.options()[2], tcp.options()[3]])
}

#[tokio::test]
async fn tcp_through_the_tunnel_is_kept_to_what_any_path_carries() {
    let mut a = spawn_node(14);
    let mut b = spawn_node_with_mtu(15, 9000);
    let everyone = Policy::default_allow_all();
    connect(&a, &b, Endpoint::Direct(b.udp), &everyone).await;
    connect(&b, &a, Endpoint::Direct(a.udp), &everyone).await;
    a.tun.inject(tcp_syn(&a, &b, false)).unwrap();
    assert_eq!(
        announced_mss(&next_packet(&mut b).await),
        1160,
        "the sender keeps what it sends to fit"
    );
    b.tun.inject(tcp_syn(&b, &a, true)).unwrap();
    assert_eq!(
        announced_mss(&next_packet(&mut a).await),
        1160,
        "the receiver keeps what it is told to fit"
    );
}

fn _dataplane_task_is_send(dataplane: Dataplane) -> impl Send {
    dataplane.run()
}
