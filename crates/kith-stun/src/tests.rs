//! Cross-module tests: RFC 5769 vectors, golden bytes, round trips, the probe.

extern crate std;

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::time::Duration;

use proptest::prelude::*;

use crate::{
    GIVE_UP_AFTER, MAX_RESPONSE_LEN, REQUEST_LEN, RETRANSMIT_SCHEDULE, StunError, StunProbe,
    TransactionId, encode_binding_request, encode_binding_response, is_stun, parse_binding_request,
    parse_binding_response,
};

/// RFC 5769 §2.2: IPv4 response with SOFTWARE, XOR-MAPPED-ADDRESS,
/// MESSAGE-INTEGRITY and FINGERPRINT.
const RFC5769_IPV4_RESPONSE: [u8; 80] = [
    0x01, 0x01, 0x00, 0x3c, 0x21, 0x12, 0xa4, 0x42, 0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86,
    0xfa, 0x87, 0xdf, 0xae, 0x80, 0x22, 0x00, 0x0b, 0x74, 0x65, 0x73, 0x74, 0x20, 0x76, 0x65, 0x63,
    0x74, 0x6f, 0x72, 0x20, 0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43,
    0x00, 0x08, 0x00, 0x14, 0x2b, 0x91, 0xf5, 0x99, 0xfd, 0x9e, 0x90, 0xc3, 0x8c, 0x74, 0x89, 0xf9,
    0x2a, 0xf9, 0xba, 0x53, 0xf0, 0x6b, 0xe7, 0xd7, 0x80, 0x28, 0x00, 0x04, 0xc0, 0x7d, 0x4c, 0x96,
];

/// RFC 5769 §2.3: the same with an IPv6 XOR-MAPPED-ADDRESS.
const RFC5769_IPV6_RESPONSE: [u8; 92] = [
    0x01, 0x01, 0x00, 0x48, 0x21, 0x12, 0xa4, 0x42, 0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86,
    0xfa, 0x87, 0xdf, 0xae, 0x80, 0x22, 0x00, 0x0b, 0x74, 0x65, 0x73, 0x74, 0x20, 0x76, 0x65, 0x63,
    0x74, 0x6f, 0x72, 0x20, 0x00, 0x20, 0x00, 0x14, 0x00, 0x02, 0xa1, 0x47, 0x01, 0x13, 0xa9, 0xfa,
    0xa5, 0xd3, 0xf1, 0x79, 0xbc, 0x25, 0xf4, 0xb5, 0xbe, 0xd2, 0xb9, 0xd9, 0x00, 0x08, 0x00, 0x14,
    0xa3, 0x82, 0x95, 0x4e, 0x4b, 0xe6, 0x7b, 0xf1, 0x17, 0x84, 0xc9, 0x7c, 0x82, 0x92, 0xc2, 0x75,
    0xbf, 0xe3, 0xed, 0x41, 0x80, 0x28, 0x00, 0x04, 0xc8, 0xfb, 0x0b, 0x4c,
];

const RFC5769_TXID: TransactionId = TransactionId([
    0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
]);

const TXID: TransactionId = TransactionId([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);

fn request(txid: TransactionId) -> [u8; REQUEST_LEN] {
    let mut out = [0; REQUEST_LEN];
    encode_binding_request(txid, &mut out);
    out
}

fn response(txid: TransactionId, observed: SocketAddr) -> std::vec::Vec<u8> {
    let mut out = [0; MAX_RESPONSE_LEN];
    encode_binding_response(txid, observed, &mut out).to_vec()
}

#[test]
fn rfc5769_ipv4_response() {
    assert!(is_stun(&RFC5769_IPV4_RESPONSE));
    let (txid, address) = parse_binding_response(&RFC5769_IPV4_RESPONSE).unwrap();
    assert_eq!(txid, RFC5769_TXID);
    assert_eq!(
        address,
        SocketAddr::new(Ipv4Addr::new(192, 0, 2, 1).into(), 32853)
    );
}

#[test]
fn rfc5769_ipv6_response() {
    let (txid, address) = parse_binding_response(&RFC5769_IPV6_RESPONSE).unwrap();
    assert_eq!(txid, RFC5769_TXID);
    let ip = Ipv6Addr::new(0x2001, 0xdb8, 0x1234, 0x5678, 0x11, 0x2233, 0x4455, 0x6677);
    assert_eq!(address, SocketAddr::new(ip.into(), 32853));
}

#[test]
fn corrupted_fingerprint_is_rejected() {
    let mut packet = RFC5769_IPV4_RESPONSE;
    // Flip a bit in the SOFTWARE value: only FINGERPRINT can notice.
    packet[24] ^= 1;
    assert_eq!(
        parse_binding_response(&packet),
        Err(StunError::BadFingerprint)
    );
}

#[test]
fn golden_binding_request() {
    let expected: [u8; REQUEST_LEN] = [
        0x00, 0x01, 0x00, 0x08, 0x21, 0x12, 0xa4, 0x42, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
        0x80, 0x28, 0x00, 0x04, 0x5b, 0x20, 0xf9, 0xcc,
    ];
    assert_eq!(request(TXID), expected);
    assert!(is_stun(&expected));
}

#[test]
fn golden_binding_response_ipv4() {
    let observed = SocketAddr::new(Ipv4Addr::new(192, 0, 2, 1).into(), 32853);
    let bytes = response(RFC5769_TXID, observed);
    // The XOR-MAPPED-ADDRESS must match RFC 5769 §2.2 byte for byte.
    assert_eq!(bytes.get(20..32), RFC5769_IPV4_RESPONSE.get(36..48));
    assert_eq!(bytes.len(), 40);
}

#[test]
fn request_response_round_trip() {
    let addresses = [
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), 41641),
        SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            1,
        ),
    ];
    for observed in addresses {
        let txid = parse_binding_request(&request(TXID)).unwrap();
        assert_eq!(txid, TXID);
        let reply = response(txid, observed);
        assert!(is_stun(&reply));
        assert_eq!(parse_binding_response(&reply), Ok((TXID, observed)));
    }
}

#[test]
fn message_types_are_not_confused() {
    let reply = response(TXID, SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 1));
    assert_eq!(
        parse_binding_request(&reply),
        Err(StunError::UnexpectedType(0x0101))
    );
    assert_eq!(
        parse_binding_response(&request(TXID)),
        Err(StunError::UnexpectedType(0x0001))
    );
}

/// Builds a response from raw attributes (no FINGERPRINT).
fn with_attributes(attributes: &[u8]) -> std::vec::Vec<u8> {
    let mut packet = std::vec![0x01, 0x01];
    packet.extend_from_slice(&u16::try_from(attributes.len()).unwrap().to_be_bytes());
    packet.extend_from_slice(&[0x21, 0x12, 0xa4, 0x42]);
    packet.extend_from_slice(&TXID.0);
    packet.extend_from_slice(attributes);
    packet
}

#[test]
fn mapped_address_fallback_and_preference() {
    let mapped = [0x00, 0x01, 0x00, 0x08, 0x00, 0x01, 0x12, 0x34, 10, 0, 0, 1];
    let packet = with_attributes(&mapped);
    let expected = SocketAddr::new(Ipv4Addr::new(10, 0, 0, 1).into(), 0x1234);
    assert_eq!(parse_binding_response(&packet), Ok((TXID, expected)));

    // XOR-MAPPED-ADDRESS wins even when it comes second.
    let mut both = mapped.to_vec();
    both.extend_from_slice(&[
        0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43,
    ]);
    let (_, address) = parse_binding_response(&with_attributes(&both)).unwrap();
    assert_eq!(
        address,
        SocketAddr::new(Ipv4Addr::new(192, 0, 2, 1).into(), 32853)
    );
}

#[test]
fn unknown_attributes() {
    let mapped = [0x00, 0x01, 0x00, 0x08, 0x00, 0x01, 0x12, 0x34, 10, 0, 0, 1];
    // Comprehension-optional, 1-byte value padded to 4: ignored.
    let mut optional = std::vec![0x80, 0x99, 0x00, 0x01, 0xff, 0, 0, 0];
    optional.extend_from_slice(&mapped);
    assert!(parse_binding_response(&with_attributes(&optional)).is_ok());
    // Comprehension-required: rejected.
    let mut required = std::vec![0x00, 0x99, 0x00, 0x00];
    required.extend_from_slice(&mapped);
    assert_eq!(
        parse_binding_response(&with_attributes(&required)),
        Err(StunError::UnknownRequiredAttribute(0x0099))
    );
}

#[test]
fn malformed_messages() {
    assert_eq!(
        parse_binding_response(&with_attributes(&[])),
        Err(StunError::MissingAddress)
    );
    // Value longer than the message.
    assert_eq!(
        parse_binding_response(&with_attributes(&[0x00, 0x01, 0x00, 0x08])),
        Err(StunError::TruncatedAttribute)
    );
    // Missing padding after a 1-byte value.
    assert_eq!(
        parse_binding_response(&with_attributes(&[
            0x80, 0x99, 0x00, 0x01, 0xff, 0, 0, 0, 0x80, 0x99, 0x00, 0x01
        ])),
        Err(StunError::TruncatedAttribute)
    );
    // Bad address family.
    assert_eq!(
        parse_binding_response(&with_attributes(&[
            0x00, 0x01, 0x00, 0x08, 0, 3, 0, 0, 0, 0, 0, 0
        ])),
        Err(StunError::BadAddress)
    );
    // Anything after FINGERPRINT.
    let mut packet = request(TXID).to_vec();
    packet.extend_from_slice(&[0x80, 0x99, 0, 0]);
    packet[3] = 12;
    assert_eq!(
        parse_binding_request(&packet),
        Err(StunError::BadFingerprint)
    );
}

#[test]
fn is_stun_rejects_non_stun() {
    let valid = request(TXID);
    assert!(!is_stun(&valid[..REQUEST_LEN - 1]));
    let mut cookie = valid;
    cookie[4] ^= 1;
    assert!(!is_stun(&cookie));
    let mut top_bits = valid;
    top_bits[0] |= 0x40;
    assert!(!is_stun(&top_bits));
    let mut length = valid;
    length[3] = 4;
    assert!(!is_stun(&length));
    // A WireGuard handshake initiation header.
    let mut wireguard = [0_u8; 148];
    wireguard[0] = 1;
    assert!(!is_stun(&wireguard));
}

const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

#[test]
fn probe_follows_retransmit_schedule() {
    let start = ms(10_000);
    let mut probe = StunProbe::new(TXID, start);
    let expected = request(TXID);
    for offset in RETRANSMIT_SCHEDULE {
        assert_eq!(probe.poll_timeout(), Some(start + offset));
        if offset > Duration::ZERO {
            assert_eq!(probe.poll_transmit(start + offset - ms(1)), None);
        }
        assert_eq!(probe.poll_transmit(start + offset), Some(&expected[..]));
        assert_eq!(probe.poll_transmit(start + offset), None);
    }
    assert_eq!(probe.poll_timeout(), Some(start + GIVE_UP_AFTER));
    assert_eq!(probe.poll_transmit(start + GIVE_UP_AFTER - ms(1)), None);
    assert!(!probe.is_timed_out());
    assert_eq!(probe.poll_transmit(start + GIVE_UP_AFTER), None);
    assert!(probe.is_timed_out());
    assert_eq!(probe.poll_timeout(), None);
}

#[test]
fn late_poll_sends_once() {
    let mut probe = StunProbe::new(TXID, Duration::ZERO);
    assert!(probe.poll_transmit(ms(350)).is_some());
    assert_eq!(probe.poll_transmit(ms(350)), None);
    assert_eq!(probe.poll_timeout(), Some(ms(700)));
}

#[test]
fn probe_matches_transaction_id() {
    let observed = SocketAddr::new(Ipv4Addr::new(198, 51, 100, 4).into(), 5000);
    let mut probe = StunProbe::new(TXID, Duration::ZERO);
    assert!(probe.poll_transmit(Duration::ZERO).is_some());

    let other = TransactionId([0xff; 12]);
    assert_eq!(probe.handle_response(&response(other, observed)), None);
    assert_eq!(probe.handle_response(b"not stun"), None);
    assert_eq!(probe.address(), None);

    assert_eq!(
        probe.handle_response(&response(TXID, observed)),
        Some(observed)
    );
    assert_eq!(probe.address(), Some(observed));
    assert_eq!(probe.handle_response(&response(TXID, observed)), None);
    assert_eq!(probe.poll_transmit(ms(100)), None);
    assert_eq!(probe.poll_timeout(), None);
}

proptest! {
    #[test]
    fn parsers_never_panic(packet in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = is_stun(&packet);
        let _ = parse_binding_request(&packet);
        let _ = parse_binding_response(&packet);
        let _ = StunProbe::new(TXID, Duration::ZERO).handle_response(&packet);
    }

    #[test]
    fn stun_shaped_input_never_panics(attributes in proptest::collection::vec(any::<u8>(), 0..64)) {
        let mut attributes = attributes;
        attributes.truncate(attributes.len() / 4 * 4);
        let packet = with_attributes(&attributes);
        prop_assert!(is_stun(&packet));
        let _ = parse_binding_response(&packet);
    }

    #[test]
    fn round_trip_any_address(ip in any::<IpAddr>(), port in any::<u16>(), txid in any::<[u8; 12]>()) {
        let observed = SocketAddr::new(ip, port);
        let txid = TransactionId(txid);
        prop_assert_eq!(parse_binding_response(&response(txid, observed)), Ok((txid, observed)));
    }
}
