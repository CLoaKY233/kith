//! End-to-end protocol tests: interoperability with boringtun, cookies, timers, robustness.

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;
    use std::time::Duration;

    use boringtun::noise::rate_limiter::RateLimiter;
    use boringtun::noise::{Tunn, TunnResult};
    use boringtun::x25519;
    use kith_wg::{
        CookieChecker, Decapsulated, Device, Encapsulated, Now, Peer, PeerConfig, RandomSource,
        StaticIdentity, Tai64N, Transmit, WgError,
    };
    use proptest::prelude::*;

    const BUFFER: usize = 2048;

    struct TestRandom(u64);

    impl RandomSource for TestRandom {
        fn fill_bytes(&mut self, buffer: &mut [u8]) -> Result<(), WgError> {
            for byte in buffer {
                self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                *byte = (z ^ (z >> 31)).to_le_bytes()[0];
            }
            Ok(())
        }
    }

    fn random(seed: u64) -> Box<dyn RandomSource + Send> {
        Box::new(TestRandom(seed))
    }

    fn at(millis: u64) -> Now {
        let monotonic = Duration::from_millis(millis);
        Now {
            monotonic,
            wall: Tai64N::from_unix(Duration::from_hours(500_000) + monotonic),
        }
    }

    fn public_of(secret: [u8; 32]) -> [u8; 32] {
        StaticIdentity::from_secret_bytes(secret).public_key()
    }

    fn peer(secret: [u8; 32], remote: [u8; 32], seed: u64) -> Peer {
        Peer::new(
            Arc::new(StaticIdentity::from_secret_bytes(secret)),
            &PeerConfig::new(public_of(remote)),
            random(seed),
        )
        .unwrap()
    }

    fn ipv4(payload: &[u8]) -> Vec<u8> {
        let total = u16::try_from(20 + payload.len()).unwrap();
        let mut packet = vec![
            0x45, 0, 0, 0, 0, 0, 0, 0, 64, 17, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2,
        ];
        packet[2..4].copy_from_slice(&total.to_be_bytes());
        packet.extend_from_slice(payload);
        packet
    }

    fn message_type(datagram: &[u8]) -> u8 {
        datagram[0]
    }

    fn bt_network(result: TunnResult<'_>) -> Vec<u8> {
        match result {
            TunnResult::WriteToNetwork(bytes) => bytes.to_vec(),
            other @ (TunnResult::Done
            | TunnResult::Err(_)
            | TunnResult::WriteToTunnelV4(..)
            | TunnResult::WriteToTunnelV6(..)) => {
                panic!("expected a datagram, got {}", describe(&other))
            }
        }
    }

    fn bt_tunnel(result: TunnResult<'_>) -> Vec<u8> {
        match result {
            TunnResult::WriteToTunnelV4(bytes, _) => bytes.to_vec(),
            other @ (TunnResult::Done
            | TunnResult::Err(_)
            | TunnResult::WriteToNetwork(_)
            | TunnResult::WriteToTunnelV6(..)) => {
                panic!("expected a packet, got {}", describe(&other))
            }
        }
    }

    fn describe(result: &TunnResult<'_>) -> String {
        match result {
            TunnResult::Done => "Done".into(),
            TunnResult::Err(error) => format!("Err({error:?})"),
            TunnResult::WriteToNetwork(_) => "WriteToNetwork".into(),
            TunnResult::WriteToTunnelV4(..) | TunnResult::WriteToTunnelV6(..) => "Tunnel".into(),
        }
    }

    /// Delivers a datagram to a kith peer and returns what it wrote.
    fn deliver(peer: &mut Peer, now: Now, datagram: &[u8]) -> (Decapsulated, Vec<u8>) {
        let mut out = vec![0; BUFFER];
        let result = peer.decapsulate(now, datagram, &mut out).unwrap();
        let len = match result {
            Decapsulated::Packet(len) | Decapsulated::Reply(len) => len,
            Decapsulated::Nothing => 0,
        };
        out.truncate(len);
        (result, out)
    }

    fn send(peer: &mut Peer, now: Now, packet: &[u8]) -> (Encapsulated, Vec<u8>) {
        let mut out = vec![0; BUFFER];
        let result = peer.encapsulate(now, packet, &mut out).unwrap();
        let len = match result {
            Encapsulated::Transport(len) | Encapsulated::HandshakeInitiation(len) => len,
            Encapsulated::Queued => 0,
        };
        out.truncate(len);
        (result, out)
    }

    fn drain(peer: &mut Peer, now: Now) -> Vec<Vec<u8>> {
        let mut sent = Vec::new();
        let mut out = vec![0; BUFFER];
        while let Transmit::Send(len) = peer.poll_transmit(now, &mut out).unwrap() {
            sent.push(out[..len].to_vec());
        }
        sent
    }

    /// Both roles, both directions, and both sides' rekeys against boringtun 0.7.1.
    #[test]
    fn interoperates_with_boringtun_in_both_roles_and_across_rekeys() {
        let (kith_secret, bt_secret) = ([0x11; 32], [0x22; 32]);
        let mut kith = peer(kith_secret, bt_secret, 1);
        let mut bt = Tunn::new(
            x25519::StaticSecret::from(bt_secret),
            x25519::PublicKey::from(public_of(kith_secret)),
            None,
            None,
            7,
            None,
        );
        let mut scratch = vec![0; BUFFER];

        let exchange = |kith: &mut Peer, bt: &mut Tunn, now: Now, label: &[u8]| {
            let mut scratch = vec![0; BUFFER];
            let to_bt = ipv4(&[label, b" kith->bt"].concat());
            let (result, datagram) = send(kith, now, &to_bt);
            assert!(matches!(result, Encapsulated::Transport(_)));
            assert_eq!(
                bt_tunnel(bt.decapsulate(None, &datagram, &mut scratch)),
                to_bt
            );

            let to_kith = ipv4(&[label, b" bt->kith, odd length!"].concat());
            let datagram = bt_network(bt.encapsulate(&to_kith, &mut scratch));
            assert_eq!(
                deliver(kith, now, &datagram),
                (Decapsulated::Packet(to_kith.len()), to_kith)
            );
        };

        // kith initiates; its queued packet rides on the confirmation.
        let first = ipv4(b"queued during handshake");
        let (result, initiation) = send(&mut kith, at(0), &first);
        assert!(matches!(result, Encapsulated::HandshakeInitiation(148)));
        let response = bt_network(bt.decapsulate(None, &initiation, &mut scratch));
        let (result, confirmation) = deliver(&mut kith, at(10), &response);
        assert!(matches!(result, Decapsulated::Reply(_)));
        assert_eq!(
            bt_tunnel(bt.decapsulate(None, &confirmation, &mut scratch)),
            first
        );
        exchange(&mut kith, &mut bt, at(20), b"session 1");

        // kith rekeys on its own once its session passes REKEY_AFTER_TIME.
        let later = at(120_500);
        let (result, datagram) = send(&mut kith, later, &ipv4(b"old session"));
        assert!(matches!(result, Encapsulated::Transport(_)));
        bt_tunnel(bt.decapsulate(None, &datagram, &mut scratch));
        let initiations = drain(&mut kith, later);
        assert_eq!(initiations.len(), 1);
        assert_eq!(message_type(&initiations[0]), 1);
        let response = bt_network(bt.decapsulate(None, &initiations[0], &mut scratch));
        let (_, keepalive) = deliver(&mut kith, later, &response);
        assert!(matches!(
            bt.decapsulate(None, &keepalive, &mut scratch),
            TunnResult::Done
        ));
        exchange(&mut kith, &mut bt, later, b"session 2");

        // boringtun initiates; kith answers and confirms the new session on first receipt.
        let initiation = bt_network(bt.format_handshake_initiation(&mut scratch, true));
        let (result, response) = deliver(&mut kith, at(120_600), &initiation);
        assert_eq!(result, Decapsulated::Reply(92));
        let keepalive = bt_network(bt.decapsulate(None, &response, &mut scratch));
        assert_eq!(
            deliver(&mut kith, at(120_610), &keepalive).0,
            Decapsulated::Nothing
        );
        exchange(&mut kith, &mut bt, at(120_620), b"session 3");
    }

    /// MAC2 under load, against boringtun in both roles, including address binding.
    #[test]
    fn cookie_replies_under_load_interoperate_with_boringtun() {
        let (kith_secret, bt_secret) = ([0x33; 32], [0x44; 32]);
        let bt_address = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));
        let mut scratch = vec![0; BUFFER];

        // boringtun responder under load; kith initiator.
        let bt_public = x25519::PublicKey::from(public_of(bt_secret));
        let mut bt = Tunn::new(
            x25519::StaticSecret::from(bt_secret),
            x25519::PublicKey::from(public_of(kith_secret)),
            None,
            None,
            9,
            Some(Arc::new(RateLimiter::new(&bt_public, 0))),
        );
        let mut kith = peer(kith_secret, bt_secret, 2);
        let packet = ipv4(b"behind a cookie");
        let (_, initiation) = send(&mut kith, at(0), &packet);
        let cookie = bt_network(bt.decapsulate(Some(bt_address), &initiation, &mut scratch));
        assert_eq!(message_type(&cookie), 3);
        assert_eq!(deliver(&mut kith, at(10), &cookie).0, Decapsulated::Nothing);
        assert!(
            drain(&mut kith, at(4_000)).is_empty(),
            "retransmit waits for REKEY_TIMEOUT"
        );
        let retry = drain(&mut kith, at(5_400));
        assert_eq!(retry.len(), 1);
        let response = bt_network(bt.decapsulate(Some(bt_address), &retry[0], &mut scratch));
        assert_eq!(message_type(&response), 2);
        let (_, confirmation) = deliver(&mut kith, at(5_410), &response);
        assert_eq!(
            bt_tunnel(bt.decapsulate(None, &confirmation, &mut scratch)),
            packet
        );

        // kith device under load (limit 0); boringtun initiator.
        let mut device = Device::new(kith_secret, CookieChecker::new(0), random(3));
        let id = device
            .add_peer(&PeerConfig::new(public_of(bt_secret)), random(4))
            .unwrap();
        let mut bt = Tunn::new(
            x25519::StaticSecret::from(bt_secret),
            x25519::PublicKey::from(public_of(kith_secret)),
            None,
            None,
            11,
            None,
        );
        let mut out = vec![0; BUFFER];
        let mut device_reply = |device: &mut Device, source: &[u8], datagram: &[u8]| {
            let output = device
                .decapsulate(at(6_000), source, datagram, &mut out)
                .unwrap();
            let Decapsulated::Reply(len) = output.result else {
                panic!("expected a reply, got {output:?}");
            };
            (output.peer, out[..len].to_vec())
        };
        let initiation = bt_network(bt.encapsulate(&packet, &mut scratch));
        let (routed, cookie) = device_reply(&mut device, b"192.0.2.7:51820", &initiation);
        assert_eq!((routed, message_type(&cookie)), (None, 3));
        assert!(matches!(
            bt.decapsulate(None, &cookie, &mut scratch),
            TunnResult::Done
        ));
        let retry = bt_network(bt.format_handshake_initiation(&mut scratch, true));

        let (_, cookie) = device_reply(&mut device, b"198.51.100.1:51820", &retry);
        assert_eq!(
            message_type(&cookie),
            3,
            "a cookie is bound to the source address"
        );

        let (routed, response) = device_reply(&mut device, b"192.0.2.7:51820", &retry);
        assert_eq!((routed, message_type(&response)), (Some(id), 2));
        let keepalive = bt_network(bt.decapsulate(None, &response, &mut scratch));
        let output = device
            .decapsulate(at(6_010), b"192.0.2.7:51820", &keepalive, &mut out)
            .unwrap();
        assert_eq!(output.peer, Some(id));
        let queued = bt_network(bt.decapsulate(None, &[], &mut scratch));
        let output = device
            .decapsulate(at(6_020), b"", &queued, &mut out)
            .unwrap();
        assert_eq!(output.result, Decapsulated::Packet(packet.len()));
        assert_eq!(&out[..packet.len()], packet.as_slice());
    }

    /// One simulated timeline through passive keepalive, `REKEY_AFTER_TIME` and `REJECT_AFTER_TIME`.
    #[test]
    fn timers_drive_keepalive_rekey_and_expiry() {
        let (a_secret, b_secret) = ([0x55; 32], [0x66; 32]);
        let mut a = peer(a_secret, b_secret, 5);
        let mut b = peer(b_secret, a_secret, 6);
        let packet = ipv4(b"tick");

        let (_, initiation) = send(&mut a, at(0), &packet);
        let (_, response) = deliver(&mut b, at(0), &initiation);
        let (_, confirmation) = deliver(&mut a, at(0), &response);
        assert_eq!(
            deliver(&mut b, at(0), &confirmation).0,
            Decapsulated::Packet(packet.len())
        );

        // b received data and sent nothing: a keepalive is due exactly KEEPALIVE_TIMEOUT later.
        assert_eq!(b.poll_timeout(), Some(Duration::from_secs(10)));
        assert_eq!(drain(&mut b, at(9_999)), Vec::<Vec<u8>>::new());
        let keepalive = drain(&mut b, at(10_000));
        assert_eq!(keepalive.len(), 1);
        assert_eq!(keepalive[0].len(), 32);
        assert_eq!(
            deliver(&mut a, at(10_000), &keepalive[0]).0,
            Decapsulated::Nothing
        );

        // Steady two-way traffic: no handshake until the session reaches REKEY_AFTER_TIME.
        let mut rekeyed_at = None;
        for second in (15..=125).step_by(5) {
            let now = at(second * 1_000);
            let (result, datagram) = send(&mut a, now, &packet);
            assert!(matches!(result, Encapsulated::Transport(_)));
            deliver(&mut b, now, &datagram);
            let (_, datagram) = send(&mut b, now, &packet);
            deliver(&mut a, now, &datagram);
            for datagram in drain(&mut a, now) {
                if message_type(&datagram) == 1 {
                    rekeyed_at.get_or_insert(second);
                    let (_, response) = deliver(&mut b, now, &datagram);
                    deliver(&mut a, now, &response);
                } else {
                    deliver(&mut b, now, &datagram);
                }
            }
            drain(&mut b, now);
        }
        assert_eq!(rekeyed_at, Some(120));

        // Now the network goes dark. The session made at 120 s works until 300 s, no longer.
        let (result, late) = send(&mut a, at(299_999), &packet);
        assert!(matches!(result, Encapsulated::Transport(_)));
        let mut out = vec![0; BUFFER];
        assert_eq!(
            b.decapsulate(at(300_000), &late, &mut out),
            Err(WgError::SessionExpired)
        );
        let (result, _) = send(&mut a, at(300_000), &packet);
        assert!(!matches!(result, Encapsulated::Transport(_)));
    }

    const A_SECRET: [u8; 32] = [0x77; 32];
    const B_SECRET: [u8; 32] = [0x88; 32];

    /// `b` holds a session with `a`; returns `b` and a valid transport message from `a`.
    /// Fresh, validly timestamped initiations from one peer are throttled per peer.
    #[test]
    fn initiation_flood_from_one_peer_is_throttled() {
        let (a, b) = ([1; 32], [2; 32]);
        let mut responder = peer(b, a, 1);
        let initiations: Vec<Vec<u8>> = [(10, 0), (11, 1), (12, 20)]
            .into_iter()
            .map(|(seed, millis)| send(&mut peer(a, b, seed), at(millis), &ipv4(b"x")).1)
            .collect();
        let mut out = vec![0; BUFFER];
        assert!(matches!(
            responder.decapsulate(at(0), &initiations[0], &mut out),
            Ok(Decapsulated::Reply(_))
        ));
        assert_eq!(
            responder.decapsulate(at(1), &initiations[1], &mut out),
            Err(WgError::Replay)
        );
        assert!(matches!(
            responder.decapsulate(at(20), &initiations[2], &mut out),
            Ok(Decapsulated::Reply(_))
        ));
    }

    fn established(seed: u64) -> (Peer, Vec<u8>) {
        let mut a = peer(A_SECRET, B_SECRET, seed);
        let mut b = peer(B_SECRET, A_SECRET, seed ^ 0xffff);
        let (_, initiation) = send(&mut a, at(0), &ipv4(b"x"));
        let (_, response) = deliver(&mut b, at(0), &initiation);
        let (_, confirmation) = deliver(&mut a, at(0), &response);
        deliver(&mut b, at(0), &confirmation);
        let (_, transport) = send(&mut a, at(1), &ipv4(b"payload"));
        (b, transport)
    }

    fn datagram_strategy() -> impl Strategy<Value = Vec<u8>> {
        (
            0_u8..6,
            prop::bool::weighted(0.9),
            prop_oneof![Just(148_usize), Just(92), Just(64), 0_usize..200],
            any::<u64>(),
        )
            .prop_map(|(kind, zero_reserved, len, seed)| {
                let mut bytes = vec![0; len];
                TestRandom(seed).fill_bytes(&mut bytes).unwrap();
                if let Some(first) = bytes.first_mut() {
                    *first = kind;
                }
                if zero_reserved {
                    for byte in bytes.iter_mut().skip(1).take(3) {
                        *byte = 0;
                    }
                }
                bytes
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// Garbage never panics, is never delivered, and never disturbs the live session.
        #[test]
        fn arbitrary_datagrams_are_rejected_without_side_effects(
            datagrams in proptest::collection::vec(datagram_strategy(), 1..8),
            seed in any::<u64>(),
        ) {
            let (mut b, transport) = established(seed);
            let mut out = vec![0; BUFFER];
            for datagram in &datagrams {
                let result = b.decapsulate(at(2), datagram, &mut out);
                prop_assert!(!matches!(result, Ok(Decapsulated::Packet(_))));
            }
            let result = b.decapsulate(at(3), &transport, &mut out);
            prop_assert!(matches!(result, Ok(Decapsulated::Packet(_))));
        }

        /// Flipping any authenticated bit of an initiation, response or transport message
        /// gets it rejected, and the untouched original is still accepted afterwards. MAC2 is
        /// excluded: it is only checked under load, which the cookie test covers.
        #[test]
        fn tampered_messages_are_rejected(
            kind in 1_u8..=4,
            position in any::<prop::sample::Index>(),
            bit in 0_u8..8,
            seed in any::<u64>(),
        ) {
            let (mut receiver, original, authenticated_len) = match kind {
                1 => {
                    let (b, _) = established(seed);
                    let (_, initiation) =
                        send(&mut peer(A_SECRET, B_SECRET, seed + 1), at(6_000), b"");
                    (b, initiation, 132)
                }
                2 | 3 => {
                    let mut a = peer(A_SECRET, B_SECRET, seed);
                    let mut b = peer(B_SECRET, A_SECRET, seed ^ 0xffff);
                    let (_, initiation) = send(&mut a, at(0), &ipv4(b"x"));
                    let (_, response) = deliver(&mut b, at(0), &initiation);
                    (a, response, 76)
                }
                _ => {
                    let (b, transport) = established(seed);
                    let len = transport.len();
                    (b, transport, len)
                }
            };
            let mut tampered = original.clone();
            tampered[position.index(authenticated_len)] ^= 1 << bit;
            let mut out = vec![0; BUFFER];
            prop_assert!(receiver.decapsulate(at(6_001), &tampered, &mut out).is_err());
            prop_assert!(receiver.decapsulate(at(6_001), &original, &mut out).is_ok());
        }
    }
}
