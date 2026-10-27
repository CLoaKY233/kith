//! Protocol tests: convergence, efficiency, hostile peers and the wire format.

extern crate std;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use kith_crypto::SigningKey;
use kith_ledger::{Ledger, NetworkParams, OpBody, SignedOp};
use kith_types::{NetworkId, OpId, Timestamp};
use proptest::prelude::*;

use crate::{
    Bound, Fingerprint, MAX_RANGE_ITEMS, MAX_ROUNDS, MAX_SYNC_MESSAGE_LEN, Message, RangeItem,
    SyncError, SyncSession, SyncStats, fingerprint,
};

// Helpers.

const NOW: Timestamp = Timestamp::from_millis(1_000);

fn founder() -> SigningKey {
    SigningKey::from_seed(&[1; 32])
}

fn network(name: &str) -> Ledger {
    let params = NetworkParams {
        ipv4: "100.64.0.0/10".parse().unwrap(),
        ipv6: "fd7a:115c:a1e0::/48".parse().unwrap(),
    };
    Ledger::create(&founder(), name, "laptop", params, &[], 1, NOW).unwrap()
}

/// A ledger holding only `ledger`'s genesis.
fn fresh_replica(ledger: &Ledger) -> Ledger {
    Ledger::new(ledger.get(&ledger.genesis_id()).cloned().unwrap()).unwrap()
}

/// Signs an admin op by the founder on top of `ledger`'s heads.
fn admin_op(ledger: &Ledger, body: OpBody) -> SignedOp {
    ledger
        .draft(founder().node_id(), body, NOW)
        .sign(&founder())
        .unwrap()
}

fn set_record(ledger: &mut Ledger, key: &str, value: Vec<u8>) -> SignedOp {
    let op = admin_op(
        ledger,
        OpBody::SetRecord {
            key: String::from(key),
            value: Some(value),
        },
    );
    ledger.insert(op.clone()).unwrap();
    op
}

fn add_device(ledger: &mut Ledger, seed: u8) -> SignedOp {
    let op = admin_op(
        ledger,
        OpBody::AddDevice {
            device: SigningKey::from_seed(&[seed; 32]).node_id(),
            name: format!("device-{seed}"),
            tags: Vec::new(),
        },
    );
    ledger.insert(op.clone()).unwrap();
    op
}

fn ids(ledger: &Ledger) -> Vec<OpId> {
    ledger.op_ids().copied().collect()
}

/// Runs `a` (initiator) against `b` in memory until both are done, checking every message
/// against the size limit. Returns both sessions' stats.
fn run(a: &mut Ledger, b: &mut Ledger) -> (SyncStats, SyncStats) {
    let mut initiator = SyncSession::new(true, a.network_id());
    let mut responder = SyncSession::new(false, b.network_id());
    for _ in 0..10_000 {
        let mut progressed = false;
        while let Some(message) = initiator.poll_transmit(a) {
            assert!(message.len() <= MAX_SYNC_MESSAGE_LEN);
            responder.handle_message(b, &message).unwrap();
            progressed = true;
        }
        while let Some(message) = responder.poll_transmit(b) {
            assert!(message.len() <= MAX_SYNC_MESSAGE_LEN);
            initiator.handle_message(a, &message).unwrap();
            progressed = true;
        }
        if !progressed {
            assert!(initiator.is_done() && responder.is_done(), "sync stalled");
            return (*initiator.stats(), *responder.stats());
        }
    }
    panic!("sync did not terminate");
}

fn assert_converged(a: &Ledger, b: &Ledger) {
    assert_eq!(ids(a), ids(b));
    assert_eq!(a.view(), b.view());
    assert_eq!((a.orphan_count(), b.orphan_count()), (0, 0));
}

// Convergence.

/// One divergent history: a shared prefix, then ops on each side. B also learned the first
/// `copied` of A's ops before branching off, so the DAGs share an inner fork too.
#[derive(Debug, Clone)]
struct History {
    shared: usize,
    a_only: Vec<(u8, u8)>,
    b_only: Vec<(u8, u8)>,
    copied: usize,
}

fn history() -> impl Strategy<Value = History> {
    (
        0..10_usize,
        prop::collection::vec((0..6_u8, any::<u8>()), 0..80),
        prop::collection::vec((0..6_u8, any::<u8>()), 0..80),
        0..80_usize,
    )
        .prop_map(|(shared, a_only, b_only, copied)| History {
            shared,
            a_only,
            b_only,
            copied,
        })
}

/// Applies one random admin op: mostly records on a few contended keys, sometimes a device.
fn random_op(ledger: &mut Ledger, side: u8, (key, value): (u8, u8)) -> SignedOp {
    if key == 0 {
        add_device(ledger, value.saturating_add(side))
    } else {
        set_record(ledger, &format!("key-{key}"), vec![side, value])
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(miri) { 4 } else { 64 }))]

    #[test]
    fn divergent_replicas_converge_in_bounded_rounds(history in history()) {
        let mut a = network("home");
        for index in 0..history.shared {
            set_record(&mut a, "shared", vec![u8::try_from(index).unwrap()]);
        }
        let mut b = a.clone();
        let a_ops: Vec<SignedOp> =
            history.a_only.iter().map(|op| random_op(&mut a, 1, *op)).collect();
        b.insert_batch(a_ops.into_iter().take(history.copied));
        for op in &history.b_only {
            random_op(&mut b, 2, *op);
        }

        let (initiator, responder) = run(&mut a, &mut b);

        assert_converged(&a, &b);
        // Under 200 ops the ranges resolve in two levels; a regression to op-by-op
        // ping-pong or a missed termination shows up here long before MAX_ROUNDS.
        prop_assert!(initiator.rounds <= 6 && responder.rounds <= 6, "{initiator:?} {responder:?}");
        // Nothing crosses the wire twice.
        prop_assert_eq!((initiator.ops_duplicate, responder.ops_duplicate), (0, 0));
    }
}

#[test]
fn replicas_with_equal_heads_finish_in_one_round_trip() {
    let mut a = network("home");
    set_record(&mut a, "printer", vec![1]);
    add_device(&mut a, 7);
    let mut b = a.clone();

    let (initiator, responder) = run(&mut a, &mut b);

    assert_eq!((initiator.messages_sent, responder.messages_sent), (1, 1));
    assert_eq!((initiator.rounds, responder.rounds), (1, 1));
}

#[test]
fn new_device_catches_up_on_a_large_ledger_within_message_limits() {
    let mut big = network("home");
    for index in 0..500_u16 {
        // Large values force the ops to be split by bytes, not just by count.
        let value = vec![u8::try_from(index % 251).unwrap(); 3_000];
        set_record(&mut big, &format!("record-{}", index % 50), value);
    }
    let mut new = fresh_replica(&big);

    let (initiator, responder) = run(&mut new, &mut big);

    assert_converged(&new, &big);
    assert_eq!(responder.ops_sent, 500, "every op sent exactly once");
    assert_eq!(
        initiator.ops_inserted, 500,
        "causal order leaves no orphans"
    );
    assert!(
        responder.messages_sent >= 4,
        "3 MB must span several messages"
    );
    // Hello, the id list round, then the two-step `Done` handshake.
    assert!(initiator.rounds <= 4, "{initiator:?}");
}

// Hostile peers.

/// A responder that has accepted a `Hello` with different heads, so it is mid-sync.
fn mid_sync(ledger: &mut Ledger) -> SyncSession {
    let mut session = SyncSession::new(false, ledger.network_id());
    let hello = Message::Hello {
        network: ledger.network_id(),
        heads: vec![OpId::from_bytes([9; 32])],
    };
    session
        .handle_message(ledger, &hello.encode().unwrap())
        .unwrap();
    session
}

proptest! {
    #[test]
    fn garbage_from_a_peer_never_panics_or_changes_the_ledger(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
        as_op in any::<bool>(),
    ) {
        let mut ledger = network("home");
        set_record(&mut ledger, "printer", vec![1]);
        let before = (ids(&ledger), ledger.view().clone());
        let mut session = mid_sync(&mut ledger);

        let message = if as_op {
            Message::Ops(vec![bytes]).encode().unwrap()
        } else {
            bytes
        };
        let _result = session.handle_message(&mut ledger, &message);

        prop_assert_eq!((ids(&ledger), ledger.view().clone()), before);
    }
}

#[test]
fn invalid_ops_are_skipped_without_stopping_sync() {
    let mut a = network("home");
    set_record(&mut a, "printer", vec![1]);
    let mut b = fresh_replica(&a);
    let mut other = network("work");
    let foreign = set_record(&mut other, "printer", vec![2]).to_bytes();
    let mut tampered = set_record(&mut a, "nas", vec![3]).to_bytes();
    if let Some(last) = tampered.last_mut() {
        *last ^= 1;
    }

    // B sees A's Hello, then a hostile Ops message, then the rest of an honest sync.
    let mut initiator = SyncSession::new(true, a.network_id());
    let mut responder = SyncSession::new(false, b.network_id());
    let hello = initiator.poll_transmit(&a).unwrap();
    responder.handle_message(&mut b, &hello).unwrap();
    let hostile = Message::Ops(vec![foreign, tampered]).encode().unwrap();
    responder.handle_message(&mut b, &hostile).unwrap();
    while !(initiator.is_done() && responder.is_done()) {
        while let Some(message) = responder.poll_transmit(&b) {
            initiator.handle_message(&mut a, &message).unwrap();
        }
        while let Some(message) = initiator.poll_transmit(&a) {
            responder.handle_message(&mut b, &message).unwrap();
        }
    }

    assert_converged(&a, &b);
    assert_eq!(responder.stats().ops_rejected, 2);
}

#[test]
fn a_peer_that_never_converges_is_cut_off_after_max_rounds() {
    let mut ledger = network("home");
    let mut session = SyncSession::new(true, ledger.network_id());
    let _hello = session.poll_transmit(&ledger).unwrap();
    let hello = Message::Hello {
        network: ledger.network_id(),
        heads: vec![OpId::from_bytes([9; 32])],
    };
    session
        .handle_message(&mut ledger, &hello.encode().unwrap())
        .unwrap();
    // The peer claims a mismatch over the whole space, forever.
    let stuck = Message::Ranges(vec![RangeItem::Fingerprint {
        upper_bound: Bound::End,
        fingerprint: Fingerprint([0; 32]),
    }])
    .encode()
    .unwrap();

    for _ in 1..MAX_ROUNDS {
        session.handle_message(&mut ledger, &stuck).unwrap();
        while session.poll_transmit(&ledger).is_some() {}
    }
    assert_eq!(
        session.handle_message(&mut ledger, &stuck),
        Err(SyncError::TooManyRounds)
    );
    assert_eq!(session.poll_transmit(&ledger), None);
}

#[test]
fn oversized_range_reply_folds_its_tail_into_one_fingerprint() {
    let mut ledger = network("home");
    for index in 0..600_u16 {
        set_record(&mut ledger, "counter", index.to_le_bytes().to_vec());
    }
    let all = ids(&ledger);
    // 17 mismatched ranges of 35 ids each: splitting all of them needs 272 > 256 items.
    let mut items: Vec<RangeItem> = all
        .chunks(35)
        .skip(1)
        .take(17)
        .map(|chunk| RangeItem::Fingerprint {
            upper_bound: Bound::Id(chunk[0]),
            fingerprint: Fingerprint([0; 32]),
        })
        .collect();
    items.push(RangeItem::Fingerprint {
        upper_bound: Bound::End,
        fingerprint: fingerprint(&all[35 * 17..]),
    });
    let mut session = mid_sync(&mut ledger);
    while session.poll_transmit(&ledger).is_some() {}

    session
        .handle_message(&mut ledger, &Message::Ranges(items).encode().unwrap())
        .unwrap();

    let reply = Message::decode(&session.poll_transmit(&ledger).unwrap()).unwrap();
    let Message::Ranges(reply) = reply else {
        panic!("expected ranges, got {reply:?}");
    };
    assert_eq!(reply.len(), MAX_RANGE_ITEMS);
    let Some(RangeItem::Fingerprint {
        fingerprint: tail, ..
    }) = reply.last()
    else {
        panic!("tail must be a fingerprint");
    };
    let Bound::Id(tail_start) = reply[MAX_RANGE_ITEMS - 2].upper_bound() else {
        panic!("tail must start inside the space");
    };
    let tail_ids: Vec<OpId> = all.iter().copied().filter(|id| *id >= tail_start).collect();
    assert_eq!(*tail, fingerprint(&tail_ids));
}

// Wire format.

#[test]
fn ranges_message_and_fingerprint_match_golden_bytes() {
    let message = Message::Ranges(vec![
        RangeItem::Fingerprint {
            upper_bound: Bound::Id(OpId::from_bytes([0x11; 32])),
            fingerprint: Fingerprint([0x22; 32]),
        },
        RangeItem::IdList {
            upper_bound: Bound::End,
            ids: vec![OpId::from_bytes([0x33; 32])],
        },
    ]);
    let mut expected = vec![b'k', b't', 0x60, 0x01, 0x01, 0x02, 0x00, 0x00];
    expected.extend([0x11; 32]);
    expected.extend([0x22; 32]);
    expected.extend([0x01, 0x01, 0x01]);
    expected.extend([0x33; 32]);

    let bytes = message.encode().unwrap();

    assert_eq!(bytes, expected);
    assert_eq!(Message::decode(&bytes).unwrap(), message);
    // Changing the fingerprint function silently turns every range into a mismatch.
    let ids = [OpId::from_bytes([0xff; 32]), OpId::from_bytes([0x01; 32])];
    assert_eq!(
        fingerprint(&ids).0,
        [
            194, 71, 124, 154, 38, 138, 210, 45, 168, 130, 105, 75, 213, 30, 60, 14, 191, 156, 32,
            102, 54, 8, 162, 143, 24, 179, 30, 90, 142, 93, 200, 107
        ]
    );
}

#[test]
fn hello_for_another_network_fails_the_session() {
    let mut ledger = network("home");
    let mut session = SyncSession::new(false, ledger.network_id());
    let found = NetworkId::from_bytes([5; 32]);
    let hello = Message::Hello {
        network: found,
        heads: Vec::new(),
    };

    let result = session.handle_message(&mut ledger, &hello.encode().unwrap());

    assert_eq!(
        result,
        Err(SyncError::WrongNetwork {
            expected: ledger.network_id(),
            found
        })
    );
    assert_eq!(session.poll_transmit(&ledger), None);
}
