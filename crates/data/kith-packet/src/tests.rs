//! Golden-vector and property tests.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "the reference checksum uses plain arithmetic on purpose"
)]

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use proptest::prelude::*;

use crate::checksum::{Checksum, checksum, update_u16};
use crate::{
    Error, FlowKey, IcmpPacket, IpPacket, Ipv4Packet, Ipv6Packet, PseudoHeader, TcpFlags,
    TcpPacket, ip_protocol,
};

/// IPv4/UDP DNS query: 192.168.1.10:53000 -> 8.8.8.8:53, `example.com A`.
const DNS_QUERY: [u8; 57] = [
    0x45, 0x00, 0x00, 0x39, 0x1c, 0x46, 0x40, 0x00, 0x40, 0x11, 0x4c, 0xac, 0xc0, 0xa8, 0x01, 0x0a,
    0x08, 0x08, 0x08, 0x08, 0xcf, 0x08, 0x00, 0x35, 0x00, 0x25, 0x75, 0x0b, 0x1a, 0x2b, 0x01, 0x00,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65,
    0x03, 0x63, 0x6f, 0x6d, 0x00, 0x00, 0x01, 0x00, 0x01,
];

/// IPv4/TCP SYN with MSS, SACK-permitted, timestamps, NOP and window scale.
const TCP_SYN: [u8; 60] = [
    0x45, 0x00, 0x00, 0x3c, 0xbe, 0xef, 0x40, 0x00, 0x40, 0x06, 0x3b, 0xf0, 0x0a, 0x00, 0x00, 0x02,
    0x5d, 0xb8, 0xd8, 0x22, 0xc9, 0x3a, 0x01, 0xbb, 0x3a, 0x1b, 0x2c, 0x3d, 0x00, 0x00, 0x00, 0x00,
    0xa0, 0x02, 0xfa, 0xf0, 0xf9, 0xa2, 0x00, 0x00, 0x02, 0x04, 0x05, 0xb4, 0x04, 0x02, 0x08, 0x0a,
    0x00, 0x01, 0xe2, 0x40, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03, 0x03, 0x07,
];

/// IPv6/UDP datagram between two ULA addresses, payload `kith`, flow label 0x12345.
const IPV6_UDP: [u8; 52] = [
    0x60, 0x01, 0x23, 0x45, 0x00, 0x0c, 0x11, 0x40, 0xfd, 0x7a, 0x11, 0x5c, 0xa1, 0xe0, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa1, 0xfd, 0x7a, 0x11, 0x5c, 0xa1, 0xe0, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb2, 0xa2, 0xa9, 0xa2, 0xa9, 0x00, 0x0c, 0x77, 0xef,
    0x6b, 0x69, 0x74, 0x68,
];

/// IPv4/ICMP echo request to 1.1.1.1, id 1, seq 7, odd-length data `kithping!`.
const ICMP_ECHO: [u8; 37] = [
    0x45, 0x00, 0x00, 0x25, 0x0a, 0xbc, 0x40, 0x00, 0x40, 0x01, 0x6c, 0x68, 0xc0, 0xa8, 0x01, 0x0a,
    0x01, 0x01, 0x01, 0x01, 0x08, 0x00, 0x18, 0x55, 0x00, 0x01, 0x00, 0x07, 0x6b, 0x69, 0x74, 0x68,
    0x70, 0x69, 0x6e, 0x67, 0x21,
];

/// RFC 1071 reference: one 16-bit word at a time, u32 accumulator.
fn reference_checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for pair in bytes.chunks(2) {
        let high = u32::from(pair[0]);
        let low = u32::from(pair.get(1).copied().unwrap_or(0));
        sum += (high << 8) | low;
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !u16::try_from(sum).unwrap()
}

#[test]
fn dns_query_golden() {
    let ip = Ipv4Packet::new_checked(&DNS_QUERY[..]).unwrap();
    assert_eq!(ip.header_len(), 20);
    assert_eq!(ip.total_len(), 57);
    assert_eq!(ip.dscp(), 0);
    assert_eq!(ip.ecn(), 0);
    assert_eq!(ip.identification(), 0x1c46);
    assert!(ip.dont_fragment());
    assert!(!ip.more_fragments());
    assert_eq!(ip.flags(), 0b010);
    assert_eq!(ip.fragment_offset(), 0);
    assert_eq!(ip.ttl(), 64);
    assert_eq!(ip.protocol(), ip_protocol::UDP);
    assert_eq!(ip.header_checksum(), 0x4cac);
    assert_eq!(ip.src(), Ipv4Addr::new(192, 168, 1, 10));
    assert_eq!(ip.dst(), Ipv4Addr::new(8, 8, 8, 8));
    assert!(ip.verify_checksum());

    let mut copy = DNS_QUERY;
    let mut ip = Ipv4Packet::new_checked(&mut copy[..]).unwrap();
    ip.fill_checksum();
    assert_eq!(copy, DNS_QUERY);
}

#[test]
fn tcp_syn_golden() {
    let ip = IpPacket::new_checked(&TCP_SYN[..]).unwrap();
    let IpPacket::V4(v4) = ip else {
        panic!("expected IPv4");
    };
    assert!(v4.verify_checksum());
    assert_eq!(v4.header_checksum(), 0x3bf0);
    assert_eq!(v4.identification(), 0xbeef);
    assert_eq!(ip.src(), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));
    assert_eq!(ip.dst(), IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)));
    assert_eq!(ip.protocol(), ip_protocol::TCP);

    let tcp = TcpPacket::new_checked(ip.payload()).unwrap();
    assert_eq!(tcp.src_port(), 51514);
    assert_eq!(tcp.dst_port(), 443);
    assert_eq!(tcp.sequence_number(), 0x3a1b_2c3d);
    assert_eq!(tcp.ack_number(), 0);
    assert_eq!(tcp.data_offset(), 10);
    assert_eq!(tcp.header_len(), 40);
    assert_eq!(tcp.flags(), TcpFlags::SYN);
    assert!(tcp.flags().contains(TcpFlags::SYN));
    assert!(!tcp.flags().contains(TcpFlags::ACK));
    assert_eq!(tcp.window(), 64240);
    assert_eq!(tcp.checksum(), 0xf9a2);
    assert_eq!(tcp.urgent_pointer(), 0);
    assert_eq!(tcp.options(), &TCP_SYN[40..]);
    assert_eq!(tcp.payload().len(), 0);
    assert!(tcp.verify_checksum(&ip.pseudo_header()));

    let mut copy = TCP_SYN;
    let mut v4 = Ipv4Packet::new_checked(&mut copy[..]).unwrap();
    v4.fill_checksum();
    TcpPacket::new_checked(v4.payload_mut())
        .unwrap()
        .fill_checksum(&ip.pseudo_header());
    assert_eq!(copy, TCP_SYN);
}

#[test]
fn icmp_echo_golden() {
    let ip = Ipv4Packet::new_checked(&ICMP_ECHO[..]).unwrap();
    assert!(ip.verify_checksum());
    assert_eq!(ip.protocol(), ip_protocol::ICMP);
    assert_eq!(ip.dst(), Ipv4Addr::new(1, 1, 1, 1));
    let icmp = IcmpPacket::new_checked(ip.payload()).unwrap();
    assert_eq!(icmp.message_type(), 8);
    assert_eq!(icmp.code(), 0);
    assert_eq!(icmp.checksum(), 0x1855);
    assert_eq!(icmp.payload(), b"\x00\x01\x00\x07kithping!");
    assert!(icmp.verify_checksum_v4());

    let mut copy = ICMP_ECHO;
    let mut ip = Ipv4Packet::new_checked(&mut copy[..]).unwrap();
    IcmpPacket::new_checked(ip.payload_mut())
        .unwrap()
        .fill_checksum_v4();
    assert_eq!(copy, ICMP_ECHO);
}

#[test]
fn icmpv6_checksum_round_trip() {
    let src = Ipv6Addr::LOCALHOST;
    let dst: Ipv6Addr = "fe80::1".parse().unwrap();
    let mut message = [128, 0, 0, 0, 0, 1, 0, 2, 0xaa];
    let mut icmp = IcmpPacket::new_checked(&mut message[..]).unwrap();
    icmp.fill_checksum_v6(src, dst);
    assert!(icmp.verify_checksum_v6(src, dst));
    assert!(!icmp.verify_checksum_v6(dst, Ipv6Addr::UNSPECIFIED));
}

#[test]
fn rfc_791_header_example() {
    // The widely quoted header from RFC 791 tutorials with checksum 0xb861.
    let header = [
        0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0xb8, 0x61, 0xc0, 0xa8, 0x00,
        0x01, 0xc0, 0xa8, 0x00, 0xc7,
    ];
    assert_eq!(checksum(&header), 0);
}

#[test]
fn flow_keys() {
    let key = FlowKey::extract(&DNS_QUERY).unwrap();
    assert_eq!(
        key,
        FlowKey {
            src: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            dst: IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            protocol: ip_protocol::UDP,
            src_port: Some(53000),
            dst_port: Some(53),
        }
    );
    let key = FlowKey::extract(&TCP_SYN).unwrap();
    assert_eq!((key.src_port, key.dst_port), (Some(51514), Some(443)));
    let key = FlowKey::extract(&IPV6_UDP).unwrap();
    assert_eq!(key.protocol, ip_protocol::UDP);
    assert_eq!(key.src_port, Some(41641));
    let key = FlowKey::extract(&ICMP_ECHO).unwrap();
    assert_eq!((key.protocol, key.src_port), (ip_protocol::ICMP, None));
}

#[test]
fn flow_key_ignores_ports_in_later_fragments() {
    let mut packet = DNS_QUERY;
    // Fragment offset 1 (8 bytes), DF clear.
    packet[6] = 0x00;
    packet[7] = 0x01;
    let key = FlowKey::extract(&packet).unwrap();
    assert_eq!((key.src_port, key.dst_port), (None, None));
}

#[test]
fn rejects_malformed_headers() {
    assert_eq!(IpPacket::new_checked(&[][..]), Err(Error::Truncated));
    assert_eq!(IpPacket::new_checked(&[0x50][..]), Err(Error::BadVersion));
    assert_eq!(
        Ipv4Packet::new_checked(&IPV6_UDP[..]),
        Err(Error::BadVersion)
    );
    assert_eq!(
        Ipv6Packet::new_checked(&DNS_QUERY[..]),
        Err(Error::BadVersion)
    );
    assert_eq!(
        Ipv4Packet::new_checked(&DNS_QUERY[..19]),
        Err(Error::Truncated)
    );
    assert_eq!(
        Ipv4Packet::new_checked(&DNS_QUERY[..56]),
        Err(Error::Truncated)
    );
    let mut bad_ihl = DNS_QUERY;
    bad_ihl[0] = 0x44;
    assert_eq!(
        Ipv4Packet::new_checked(&bad_ihl[..]),
        Err(Error::BadHeaderLength)
    );
    let mut bad_total = DNS_QUERY;
    bad_total[3] = 0x10;
    assert_eq!(
        Ipv4Packet::new_checked(&bad_total[..]),
        Err(Error::BadTotalLength)
    );
    assert_eq!(
        Ipv6Packet::new_checked(&IPV6_UDP[..51]),
        Err(Error::Truncated)
    );
    let mut short_offset = TCP_SYN;
    short_offset[32] = 0x40;
    assert_eq!(
        TcpPacket::new_checked(&short_offset[20..]),
        Err(Error::BadHeaderLength)
    );
    assert_eq!(
        TcpPacket::new_checked(&TCP_SYN[20..59]),
        Err(Error::Truncated)
    );
    assert_eq!(
        IcmpPacket::new_checked(&[8, 0, 0][..]),
        Err(Error::Truncated)
    );
    assert_eq!(FlowKey::extract(&DNS_QUERY[..22]), Err(Error::Truncated));
}

#[test]
fn trailing_link_padding_is_ignored() {
    let mut padded = [0u8; 64];
    padded[..57].copy_from_slice(&DNS_QUERY);
    let ip = Ipv4Packet::new_checked(&padded[..]).unwrap();
    assert_eq!(ip.payload().len(), 37);
}

#[test]
fn setters_keep_header_checksum_valid() {
    let mut packet = DNS_QUERY;
    let mut ip = Ipv4Packet::new_checked(&mut packet[..]).unwrap();
    ip.set_src(Ipv4Addr::new(100, 64, 0, 1));
    ip.set_dst(Ipv4Addr::new(100, 64, 0, 2));
    ip.set_ttl(1);
    assert!(ip.verify_checksum());
    assert_eq!(ip.src(), Ipv4Addr::new(100, 64, 0, 1));
    assert_eq!(ip.dst(), Ipv4Addr::new(100, 64, 0, 2));
    assert_eq!(ip.ttl(), 1);
    assert_eq!(ip.protocol(), ip_protocol::UDP);

    let mut v6 = IPV6_UDP;
    let mut ip = Ipv6Packet::new_checked(&mut v6[..]).unwrap();
    ip.set_hop_limit(3);
    assert_eq!(ip.hop_limit(), 3);
}

#[test]
fn odd_length_checksums() {
    assert_eq!(checksum(&[]), 0xffff);
    assert_eq!(checksum(&[0x01]), !0x0100);
    assert_eq!(checksum(&[0x01, 0x02, 0x03]), !0x0402);
    for length in 0..=40u8 {
        let bytes: std::vec::Vec<u8> = (0..length)
            .map(|i: u8| i.wrapping_mul(37).wrapping_add(11))
            .collect();
        assert_eq!(
            checksum(&bytes),
            reference_checksum(&bytes),
            "length {length}"
        );
    }
}

#[test]
fn incremental_update_matches_for_simple_word() {
    let mut header = [
        0x45, 0, 0, 20, 0, 0, 0, 0, 64, 17, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2,
    ];
    let full = checksum(&header);
    header[8] = 63;
    let after = checksum(&header);
    assert_eq!(update_u16(full, 0x4011, 0x3f11), after);
    let mut sum = Checksum::new();
    sum.add_bytes(&header);
    assert_eq!(sum.finish(), after);
}

fn ipv4_header() -> impl Strategy<Value = [u8; 20]> {
    any::<[u8; 20]>().prop_map(|mut header| {
        header[0] = 0x45;
        header[2] = 0;
        header[3] = 20;
        header
    })
}

proptest! {
    #[test]
    fn checksum_matches_reference(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        prop_assert_eq!(checksum(&bytes), reference_checksum(&bytes));
    }

    #[test]
    fn incremental_equals_full_recompute(
        header in ipv4_header(),
        src in any::<[u8; 4]>(),
        dst in any::<[u8; 4]>(),
        ttl in any::<u8>(),
    ) {
        let mut packet = header;
        let mut ip = Ipv4Packet::new_checked(&mut packet[..]).unwrap();
        ip.fill_checksum();
        ip.set_src(Ipv4Addr::from(src));
        ip.set_dst(Ipv4Addr::from(dst));
        ip.set_ttl(ttl);
        let incremental = ip.header_checksum();
        ip.fill_checksum();
        prop_assert_eq!(incremental, ip.header_checksum());
    }

    #[test]
    fn parsers_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..128)) {
        let pseudo = PseudoHeader::V4 { src: Ipv4Addr::LOCALHOST, dst: Ipv4Addr::LOCALHOST };
        if let Ok(ip) = IpPacket::new_checked(&bytes[..]) {
            let _ = (ip.src(), ip.dst(), ip.protocol(), ip.payload(), ip.pseudo_header());
        }
        if let Ok(ip) = Ipv4Packet::new_checked(&bytes[..]) {
            let _ = (ip.verify_checksum(), ip.payload(), ip.dscp(), ip.ecn(), ip.fragment_offset());
        }
        if let Ok(ip) = Ipv6Packet::new_checked(&bytes[..]) {
            let _ = (ip.payload(), ip.flow_label(), ip.traffic_class());
        }
        if let Ok(tcp) = TcpPacket::new_checked(&bytes[..]) {
            let _ = (tcp.payload(), tcp.options(), tcp.verify_checksum(&pseudo));
        }
        if let Ok(icmp) = IcmpPacket::new_checked(&bytes[..]) {
            let _ = (icmp.payload(), icmp.verify_checksum_v4());
        }
        let _ = FlowKey::extract(&bytes);
    }

    #[test]
    fn ip_like_input_never_panics(
        version in prop_oneof![Just(0x40u8), Just(0x60u8)],
        rest in proptest::collection::vec(any::<u8>(), 0..96),
    ) {
        let mut bytes = rest;
        bytes.insert(0, version | 0x05);
        let _ = FlowKey::extract(&bytes);
        let mut copy = bytes.clone();
        if let Ok(mut ip) = Ipv4Packet::new_checked(&mut copy[..]) {
            ip.set_ttl(1);
            ip.fill_checksum();
            prop_assert!(ip.verify_checksum());
        }
    }
}

/// A TCP segment with `flags` and `options` (padded with NOPs to a word), inside IPv4 or
/// IPv6, with valid checksums.
fn tcp_segment(v6: bool, flags: u8, options: &[u8]) -> std::vec::Vec<u8> {
    let mut tcp = std::vec![
        0x30, 0x39, 0x01, 0xbb, 0, 0, 0, 1, 0, 0, 0, 0, 0, flags, 0xff, 0xff, 0, 0, 0, 0
    ];
    tcp.extend_from_slice(options);
    while tcp.len() % 4 != 0 {
        tcp.push(1);
    }
    tcp[12] = u8::try_from(tcp.len() / 4).unwrap() << 4;
    let len = u16::try_from(tcp.len()).unwrap();
    let mut packet = if v6 {
        let mut ip = std::vec![0x60, 0, 0, 0, 0, 0, ip_protocol::TCP, 64];
        ip[4..6].copy_from_slice(&len.to_be_bytes());
        ip.extend_from_slice(&Ipv6Addr::new(0xfd7a, 0, 0, 0, 0, 0, 0, 2).octets());
        ip.extend_from_slice(&Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1).octets());
        ip
    } else {
        let mut ip = std::vec![
            0x45,
            0,
            0,
            0,
            0,
            0,
            0x40,
            0,
            64,
            ip_protocol::TCP,
            0,
            0,
            100,
            64,
            0,
            2,
            93,
            184,
            216,
            34
        ];
        ip[2..4].copy_from_slice(&(len + 20).to_be_bytes());
        ip
    };
    let header = packet.len();
    packet.extend_from_slice(&tcp);
    if !v6 {
        Ipv4Packet::new_checked(&mut packet[..])
            .unwrap()
            .fill_checksum();
    }
    let pseudo = IpPacket::new_checked(&packet[..]).unwrap().pseudo_header();
    TcpPacket::new_checked(&mut packet[header..])
        .unwrap()
        .fill_checksum(&pseudo);
    packet
}

fn tcp_checksum_ok(packet: &[u8]) -> bool {
    let ip = IpPacket::new_checked(packet).unwrap();
    let header = packet.len() - ip.payload().len();
    TcpPacket::new_checked(&packet[header..])
        .unwrap()
        .verify_checksum(&ip.pseudo_header())
}

fn mss_of(packet: &[u8]) -> Option<u16> {
    let ip = IpPacket::new_checked(packet).unwrap();
    let options = TcpPacket::new_checked(ip.payload())
        .unwrap()
        .options()
        .to_vec();
    let mut at = 0;
    while at < options.len() {
        match options[at] {
            0 => return None,
            1 => at += 1,
            2 => return Some(u16::from_be_bytes([options[at + 2], options[at + 3]])),
            _ => at += usize::from(options[at + 1]),
        }
    }
    None
}

#[test]
fn a_syn_is_clamped_to_fit_the_tunnel_in_both_families_and_any_alignment() {
    let mss_1460 = [2, 4, 0x05, 0xb4];
    for v6 in [false, true] {
        let max = if v6 { 1140 } else { 1160 };
        for lead in [&[][..], &[1][..], &[1, 1, 1][..], &[4, 2, 1][..]] {
            let options: std::vec::Vec<u8> = lead.iter().chain(&mss_1460).copied().collect();
            for flags in [0x02, 0x12] {
                let mut packet = tcp_segment(v6, flags, &options);
                assert!(crate::needs_mss_clamp(&packet, 1200));
                assert!(crate::clamp_mss(&mut packet, 1200));
                assert_eq!(mss_of(&packet), Some(max), "v6 {v6}, lead {lead:?}");
                assert!(tcp_checksum_ok(&packet), "v6 {v6}, lead {lead:?}");
                assert!(!crate::clamp_mss(&mut packet, 1200), "clamping twice");
            }
        }
    }
    let mut golden = TCP_SYN;
    assert!(crate::clamp_mss(&mut golden, 1200));
    assert!(tcp_checksum_ok(&golden));
}

#[test]
fn only_a_syn_with_a_large_mss_is_touched() {
    let cases: [(bool, u8, &[u8]); 5] = [
        (false, 0x10, &[2, 4, 0x05, 0xb4]),
        (false, 0x02, &[2, 4, 0x04, 0x00]),
        (true, 0x02, &[4, 2]),
        (false, 0x02, &[0, 2, 4, 0x05, 0xb4]),
        (false, 0x02, &[2, 3, 0x05]),
    ];
    for (v6, flags, options) in cases {
        let packet = tcp_segment(v6, flags, options);
        let mut copy = packet.clone();
        assert!(!crate::needs_mss_clamp(&packet, 1200), "{options:?}");
        assert!(!crate::clamp_mss(&mut copy, 1200));
        assert_eq!(copy, packet);
    }
    let mut fragment = tcp_segment(false, 0x02, &[2, 4, 0x05, 0xb4]);
    fragment[6..8].copy_from_slice(&0x0010_u16.to_be_bytes());
    assert!(!crate::clamp_mss(&mut fragment.clone(), 1200));
    let mut extension = tcp_segment(true, 0x02, &[2, 4, 0x05, 0xb4]);
    extension[6] = 0;
    assert!(!crate::clamp_mss(&mut extension, 1200));
    assert!(!crate::clamp_mss(&mut DNS_QUERY.clone(), 1200));
}

proptest! {
    #[test]
    fn clamping_keeps_the_checksum_valid_and_never_raises_the_mss(
        v6 in any::<bool>(),
        lead in proptest::collection::vec(prop_oneof![Just(1_u8)], 0..6),
        mss in any::<u16>(),
        mtu in 1100_u16..1500,
    ) {
        let mut options = lead;
        options.extend_from_slice(&[2, 4]);
        options.extend_from_slice(&mss.to_be_bytes());
        let mut packet = tcp_segment(v6, 0x02, &options);
        crate::clamp_mss(&mut packet, mtu);
        let max = mtu - if v6 { 60 } else { 40 };
        prop_assert_eq!(mss_of(&packet), Some(mss.min(max)));
        prop_assert!(tcp_checksum_ok(&packet));
    }

    #[test]
    fn clamping_never_panics(mut bytes in proptest::collection::vec(any::<u8>(), 0..128), mtu in any::<u16>()) {
        let _ = crate::needs_mss_clamp(&bytes, mtu);
        let _ = crate::clamp_mss(&mut bytes, mtu);
    }
}
