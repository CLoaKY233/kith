//! Tests for every validity rule, the structural checks and the wire format.

extern crate std;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use kith_crypto::{Signature, SigningKey};
use kith_types::{NetworkId, NodeId, OpId, Timestamp};
use kith_wire::envelope;
use proptest::prelude::*;

use crate::op::{OP_SIGNATURE_CONTEXT, OP_VERSION, SIGNED_OP_TYPE, UNSIGNED_OP_TYPE};
use crate::{
    InsertOutcome, JoinProof, Ledger, LedgerError, MAX_NAME_LEN, MAX_ORPHANS, MAX_PARENTS,
    MAX_RECORD_KEY_LEN, MAX_RECORD_VALUE_LEN, MAX_TAGS, NetworkParams, OpBody, SignedOp,
    UnsignedOp,
};

// Helpers.

fn key(seed: u8) -> SigningKey {
    SigningKey::from_seed(&[seed; 32])
}

fn id(seed: u8) -> NodeId {
    key(seed).node_id()
}

fn params() -> NetworkParams {
    NetworkParams {
        ipv4: "100.64.0.0/10".parse().unwrap(),
        ipv6: "fd7a:115c:a1e0::/48".parse().unwrap(),
    }
}

const NOW: Timestamp = Timestamp::from_millis(1_000);

/// A network founded by key 1, with the given extra admins and threshold.
fn network(extra_admins: &[u8], threshold: u8) -> Ledger {
    let extra: Vec<NodeId> = extra_admins.iter().map(|seed| id(*seed)).collect();
    Ledger::create(&key(1), "home", "laptop", params(), &extra, threshold, NOW).unwrap()
}

/// An op by `author` on top of `ledger`'s heads.
fn op(ledger: &Ledger, author: &SigningKey, body: OpBody) -> SignedOp {
    op_at(ledger, author, body, NOW)
}

fn op_at(ledger: &Ledger, author: &SigningKey, body: OpBody, timestamp: Timestamp) -> SignedOp {
    ledger
        .draft(author.node_id(), body, timestamp)
        .sign(author)
        .unwrap()
}

/// Inserts an op that must be stored, returning its id.
fn put(ledger: &mut Ledger, op: SignedOp) -> OpId {
    let id = op.id();
    assert_eq!(ledger.insert(op), Ok(InsertOutcome::Inserted));
    id
}

/// Drafts, signs and inserts an op.
fn act(ledger: &mut Ledger, author: &SigningKey, body: OpBody) -> OpId {
    let signed = op(ledger, author, body);
    put(ledger, signed)
}

/// Inserts every op of `from` into `into`.
fn sync(into: &mut Ledger, from: &Ledger) {
    let ops: Vec<SignedOp> = from
        .op_ids()
        .filter_map(|id| from.get(id).cloned())
        .collect();
    for op in ops {
        into.insert(op).unwrap();
    }
}

fn add_device(seed: u8, name: &str) -> OpBody {
    OpBody::AddDevice {
        device: id(seed),
        name: String::from(name),
        tags: Vec::new(),
    }
}

fn create_invite(seed: u8, max_uses: u16, expires: u64) -> OpBody {
    OpBody::CreateInvite {
        invite_key: key(seed).verifying_key(),
        expires: Timestamp::from_millis(expires),
        max_uses,
        tags: vec![String::from("guest")],
    }
}

fn join(ledger: &Ledger, invite: u8, device: u8) -> OpBody {
    let proof =
        JoinProof::create(ledger.network_id(), &key(invite), &key(device), "phone").unwrap();
    OpBody::Join(proof)
}

fn set_record(record: &str, value: Option<&[u8]>) -> OpBody {
    OpBody::SetRecord {
        key: String::from(record),
        value: value.map(<[u8]>::to_vec),
    }
}

fn is_member(ledger: &Ledger, seed: u8) -> bool {
    ledger.view().is_member(&id(seed))
}

/// Signs an unchecked unsigned op with `signers` in the given order, for crafting malformed ops.
fn raw(op: &UnsignedOp, signers: &[&SigningKey]) -> Vec<u8> {
    let bytes = envelope::encode(UNSIGNED_OP_TYPE, OP_VERSION, op).unwrap();
    raw_with(&bytes, signers)
}

fn raw_with(unsigned: &[u8], signers: &[&SigningKey]) -> Vec<u8> {
    let signatures: Vec<(NodeId, Signature)> = signers
        .iter()
        .map(|signer| {
            (
                signer.node_id(),
                signer.sign(OP_SIGNATURE_CONTEXT, unsigned),
            )
        })
        .collect();
    envelope::encode(SIGNED_OP_TYPE, OP_VERSION, &(unsigned.to_vec(), signatures)).unwrap()
}

fn decode_err(bytes: &[u8]) -> LedgerError {
    SignedOp::from_bytes(bytes).unwrap_err()
}

// Genesis and basics.

#[test]
fn founder_is_sole_device_and_network_id_is_genesis_hash() {
    let ledger = network(&[2], 1);
    let view = ledger.view();
    assert_eq!(view.network_id().to_bytes(), ledger.genesis_id().to_bytes());
    assert_eq!(view.name(), "home");
    assert_eq!(view.params(), &params());
    assert_eq!(view.admins().iter().copied().collect::<Vec<_>>(), {
        let mut admins = vec![id(1), id(2)];
        admins.sort();
        admins
    });
    assert_eq!(view.threshold(), 1);
    // Only the founder is a device; extra admins are not.
    assert_eq!(view.devices().len(), 1);
    let founder = view.devices().get(&id(1)).unwrap();
    assert_eq!(founder.name, "laptop");
    assert_eq!(founder.admitted_by, ledger.genesis_id());
    assert_eq!(founder.admitted_at_lamport, 0);
    assert_eq!(ledger.heads(), &[ledger.genesis_id()]);
    assert_eq!(view.heads(), ledger.heads());
    assert_eq!(ledger.len(), 1);
    assert!(!ledger.is_empty());
}

#[test]
fn draft_merges_at_most_max_parents_heads() {
    let base = network(&[], 1);
    let mut ledger = base.clone();
    for seed in 0..20_u8 {
        let concurrent = op(&base, &key(1), add_device(100 + seed, "d"));
        put(&mut ledger, concurrent);
    }
    assert_eq!(ledger.heads().len(), 20);
    let draft = ledger.draft(id(1), add_device(5, "x"), NOW);
    assert_eq!(draft.parents.len(), MAX_PARENTS);
    assert_eq!(draft.parents.as_slice(), &ledger.heads()[..MAX_PARENTS]);
    let merged = draft.sign(&key(1)).unwrap();
    put(&mut ledger, merged);
    assert_eq!(ledger.heads().len(), 20 - MAX_PARENTS + 1);
}

// Rule 1: authorization.

#[test]
fn non_admin_ops_are_stored_but_excluded_from_view() {
    let mut ledger = network(&[], 1);
    // Key 3 is not an admin.
    let stranger = op(&ledger, &key(3), add_device(5, "nas"));
    put(&mut ledger, stranger.clone());
    assert!(ledger.contains(&stranger.id()));
    assert!(!is_member(&ledger, 5));
    let record = op(&ledger, &key(3), set_record("dns/nas", Some(b"1")));
    put(&mut ledger, record);
    assert!(ledger.view().records().is_empty());
    // A non-admin cannot make itself admin either.
    let grab = op(&ledger, &key(3), OpBody::AddAdmin { admin: id(3) });
    put(&mut ledger, grab);
    assert!(!ledger.view().admins().contains(&id(3)));
}

#[test]
fn authority_ops_respect_threshold_and_admin_count_preconditions() {
    let mut ledger = network(&[], 1);
    act(&mut ledger, &key(1), OpBody::AddAdmin { admin: id(2) });
    assert!(ledger.view().admins().contains(&id(2)));
    // The new admin can act.
    act(&mut ledger, &key(2), add_device(5, "nas"));
    assert!(is_member(&ledger, 5));
    act(&mut ledger, &key(1), OpBody::SetThreshold { threshold: 2 });
    assert_eq!(ledger.view().threshold(), 2);
    // Threshold above the admin count is not a valid change.
    let mut too_high = op(&ledger, &key(1), OpBody::SetThreshold { threshold: 3 });
    too_high.add_signature(&key(2)).unwrap();
    put(&mut ledger, too_high);
    assert_eq!(ledger.view().threshold(), 2);
    // Removing an admin below the threshold is not allowed either.
    let mut below = op(&ledger, &key(1), OpBody::RemoveAdmin { admin: id(2) });
    below.add_signature(&key(2)).unwrap();
    put(&mut ledger, below);
    assert!(ledger.view().admins().contains(&id(2)));
}

#[test]
fn threshold_two_applies_only_once_cosignatures_merge() {
    let mut ledger = network(&[2], 2);
    let single = op(&ledger, &key(1), OpBody::AddAdmin { admin: id(3) });
    put(&mut ledger, single.clone());
    assert!(!ledger.view().admins().contains(&id(3)));

    // A non-admin co-signature does not count.
    let mut stranger = single.clone();
    stranger.add_signature(&key(9)).unwrap();
    assert_eq!(ledger.insert(stranger), Ok(InsertOutcome::SignaturesAdded));
    assert!(!ledger.view().admins().contains(&id(3)));

    // The second admin's co-signature arrives as another copy of the same op.
    let mut cosigned = single;
    cosigned.add_signature(&key(2)).unwrap();
    cosigned.add_signature(&key(2)).unwrap();
    assert_eq!(cosigned.signatures().len(), 2);
    assert_eq!(
        ledger.insert(cosigned.clone()),
        Ok(InsertOutcome::SignaturesAdded)
    );
    assert!(ledger.view().admins().contains(&id(3)));
    assert_eq!(ledger.insert(cosigned), Ok(InsertOutcome::Duplicate));
    assert_eq!(
        ledger.get(&ledger.heads()[0]).unwrap().signatures().len(),
        3
    );

    // Admin ops still need only one admin.
    act(&mut ledger, &key(3), add_device(5, "nas"));
    assert!(is_member(&ledger, 5));
}

// Rule 2: remove-wins.

#[test]
fn concurrent_admin_removal_invalidates_admin_ops() {
    let mut base = network(&[2], 1);
    // An op by admin 2 that the removal will have seen stays valid.
    act(&mut base, &key(2), set_record("before", Some(b"kept")));

    let removal = op(&base, &key(1), OpBody::RemoveAdmin { admin: id(2) });
    let concurrent_add = op(&base, &key(2), add_device(5, "nas"));
    let concurrent_record = op(&base, &key(2), set_record("during", Some(b"lost")));
    let mut ledger = base.clone();
    put(&mut ledger, concurrent_add);
    put(&mut ledger, concurrent_record);
    assert!(is_member(&ledger, 5));
    put(&mut ledger, removal);

    assert!(!ledger.view().admins().contains(&id(2)));
    assert!(!is_member(&ledger, 5));
    assert_eq!(ledger.view().records().get("before").unwrap(), b"kept");
    assert!(!ledger.view().records().contains_key("during"));
}

#[test]
fn removed_admin_regains_power_only_after_readd() {
    let mut ledger = network(&[2], 1);
    act(&mut ledger, &key(1), OpBody::RemoveAdmin { admin: id(2) });
    act(&mut ledger, &key(2), add_device(5, "nas"));
    assert!(!is_member(&ledger, 5));
    act(&mut ledger, &key(1), OpBody::AddAdmin { admin: id(2) });
    act(&mut ledger, &key(2), add_device(6, "tv"));
    assert!(is_member(&ledger, 6));
    assert!(!is_member(&ledger, 5));
}

#[test]
fn concurrent_device_removal_beats_add() {
    let mut base = network(&[2], 1);
    act(&mut base, &key(1), add_device(5, "nas"));
    // Admin 1 re-adds (renames) the device while admin 2 removes it.
    let add = op(&base, &key(1), add_device(5, "nas2"));
    let remove = op(&base, &key(2), OpBody::RemoveDevice { device: id(5) });
    let mut ledger = base.clone();
    put(&mut ledger, add);
    put(&mut ledger, remove);
    assert!(!is_member(&ledger, 5));
}

#[test]
fn concurrent_sponsor_removal_invalidates_join() {
    let mut base = network(&[], 1);
    act(&mut base, &key(1), add_device(5, "sponsor"));
    act(&mut base, &key(1), create_invite(50, 5, 10_000));
    let joined = op(&base, &key(5), join(&base, 50, 60));
    let removal = op(&base, &key(1), OpBody::RemoveDevice { device: id(5) });

    let mut without_removal = base.clone();
    put(&mut without_removal, joined.clone());
    assert!(is_member(&without_removal, 60));

    let mut ledger = base;
    put(&mut ledger, joined);
    put(&mut ledger, removal);
    assert!(!is_member(&ledger, 5));
    assert!(!is_member(&ledger, 60));
}

#[test]
fn readd_after_removal_works() {
    let mut ledger = network(&[], 1);
    act(&mut ledger, &key(1), add_device(5, "nas"));
    act(&mut ledger, &key(1), OpBody::RemoveDevice { device: id(5) });
    assert!(!is_member(&ledger, 5));
    let readd = act(&mut ledger, &key(1), add_device(5, "nas-again"));
    let device = ledger.view().devices().get(&id(5)).unwrap();
    assert_eq!(device.name, "nas-again");
    assert_eq!(device.admitted_by, readd);
    assert_eq!(device.admitted_at_lamport, 3);
}

#[test]
fn removed_then_readded_sponsor_can_sponsor_again() {
    let mut ledger = network(&[], 1);
    act(&mut ledger, &key(1), add_device(5, "sponsor"));
    act(&mut ledger, &key(1), create_invite(50, 5, 10_000));
    act(&mut ledger, &key(1), OpBody::RemoveDevice { device: id(5) });
    let early = op(&ledger, &key(5), join(&ledger, 50, 60));
    put(&mut ledger, early);
    assert!(!is_member(&ledger, 60));
    act(&mut ledger, &key(1), add_device(5, "sponsor"));
    let late = op(&ledger, &key(5), join(&ledger, 50, 61));
    put(&mut ledger, late);
    assert!(is_member(&ledger, 61));
}

#[test]
fn mutual_concurrent_device_removals_both_apply() {
    // Admins 1 and 2 each run a device; each removes the other's device concurrently.
    let mut base = network(&[2], 1);
    act(&mut base, &key(1), add_device(2, "admin-two"));
    let first = op(&base, &key(1), OpBody::RemoveDevice { device: id(2) });
    let second = op(&base, &key(2), OpBody::RemoveDevice { device: id(1) });
    let mut ledger = base.clone();
    put(&mut ledger, first);
    put(&mut ledger, second);
    assert!(!is_member(&ledger, 1));
    assert!(!is_member(&ledger, 2));
}

#[test]
fn mutual_concurrent_admin_removals_fork_and_one_applies() {
    // Authority ops form a linear chain, so two concurrent removals are a fork: the one
    // with the lowest OpId applies, and the loser's ops concurrent with it are invalid.
    let base = network(&[2, 3], 1);
    let by_one = op(&base, &key(1), OpBody::RemoveAdmin { admin: id(2) });
    let by_two = op(&base, &key(2), OpBody::RemoveAdmin { admin: id(1) });
    let (winner, removed) = if by_one.id() < by_two.id() {
        (by_one.id(), id(2))
    } else {
        (by_two.id(), id(1))
    };
    let mut ledger = base;
    put(&mut ledger, by_one);
    put(&mut ledger, by_two);
    let admins = ledger.view().admins();
    assert!(!admins.contains(&removed));
    assert_eq!(admins.len(), 2);
    assert!(ledger.contains(&winner));
}

// Stage A: forks.

#[test]
fn authority_fork_keeps_lowest_op_id_and_drops_dependents() {
    let base = network(&[2], 1);
    let mut left = base.clone();
    let mut right = base.clone();
    let left_op = op(&left, &key(1), OpBody::AddAdmin { admin: id(3) });
    let right_op = op(&right, &key(2), OpBody::AddAdmin { admin: id(4) });
    put(&mut left, left_op.clone());
    put(&mut right, right_op.clone());
    // Each new admin acts on its own branch: an admin op and an authority op.
    act(&mut left, &key(3), add_device(30, "three"));
    act(&mut left, &key(3), OpBody::AddAdmin { admin: id(5) });
    act(&mut right, &key(4), add_device(40, "four"));
    act(&mut right, &key(4), OpBody::AddAdmin { admin: id(6) });

    let left_wins = left_op.id() < right_op.id();
    let mut merged = base;
    sync(&mut merged, &left);
    sync(&mut merged, &right);
    let admins = merged.view().admins();
    assert_eq!(admins.contains(&id(3)), left_wins);
    assert_eq!(admins.contains(&id(5)), left_wins);
    assert_eq!(admins.contains(&id(4)), !left_wins);
    assert_eq!(admins.contains(&id(6)), !left_wins);
    assert_eq!(is_member(&merged, 30), left_wins);
    assert_eq!(is_member(&merged, 40), !left_wins);

    // Same result whichever branch arrives first.
    let mut reversed = network(&[2], 1);
    sync(&mut reversed, &right);
    sync(&mut reversed, &left);
    assert_eq!(reversed.view(), merged.view());
}

#[test]
fn authority_op_on_merged_fork_extends_the_winner() {
    let base = network(&[2], 1);
    let mut ledger = base.clone();
    put(
        &mut ledger,
        op(&base, &key(1), OpBody::AddAdmin { admin: id(3) }),
    );
    put(
        &mut ledger,
        op(&base, &key(2), OpBody::AddAdmin { admin: id(4) }),
    );
    // An authority op on top of both branches extends the winner.
    act(&mut ledger, &key(1), OpBody::AddAdmin { admin: id(7) });
    assert!(ledger.view().admins().contains(&id(7)));
    assert_eq!(ledger.view().admins().len(), 4);
}

// Invites.

#[test]
fn joined_device_gets_invite_tags_and_can_sponsor() {
    let mut ledger = network(&[], 1);
    act(&mut ledger, &key(1), create_invite(50, 2, 10_000));
    assert_eq!(ledger.view().invites().len(), 1);
    let body = join(&ledger, 50, 60);
    let join_id = act(&mut ledger, &key(1), body);
    let device = ledger.view().devices().get(&id(60)).unwrap();
    assert_eq!(device.name, "phone");
    assert_eq!(device.tags, vec![String::from("guest")]);
    assert_eq!(device.admitted_by, join_id);
    let invite = ledger
        .view()
        .invites()
        .get(&key(50).verifying_key().to_bytes())
        .unwrap();
    assert_eq!(invite.uses, 1);
    assert_eq!(invite.max_uses, 2);
    // A joined device can sponsor others.
    let body = join(&ledger, 50, 61);
    act(&mut ledger, &key(60), body);
    assert!(is_member(&ledger, 61));
    // The invite is now used up.
    assert!(ledger.view().invites().is_empty());
}

#[test]
fn join_needs_member_sponsor_and_known_invite() {
    let mut ledger = network(&[], 1);
    // Unknown invite.
    let body = join(&ledger, 50, 60);
    act(&mut ledger, &key(1), body);
    assert!(!is_member(&ledger, 60));
    act(&mut ledger, &key(1), create_invite(50, 5, 10_000));
    // Non-member sponsor.
    let body = join(&ledger, 50, 61);
    act(&mut ledger, &key(9), body);
    assert!(!is_member(&ledger, 61));
    // An invite created by a non-admin is not an invite.
    act(&mut ledger, &key(9), create_invite(51, 5, 10_000));
    let body = join(&ledger, 51, 62);
    act(&mut ledger, &key(1), body);
    assert!(!is_member(&ledger, 62));
}

#[test]
fn join_must_have_the_invite_in_its_past() {
    let base = network(&[], 1);
    let create = op(&base, &key(1), create_invite(50, 5, 10_000));
    let concurrent_join = op(&base, &key(1), join(&base, 50, 60));
    let mut ledger = base;
    put(&mut ledger, create);
    put(&mut ledger, concurrent_join);
    assert!(!is_member(&ledger, 60));
}

#[test]
fn invite_expiry_is_inclusive() {
    let mut ledger = network(&[], 1);
    act(&mut ledger, &key(1), create_invite(50, 5, 2_000));
    let on_time = op_at(
        &ledger,
        &key(1),
        join(&ledger, 50, 60),
        Timestamp::from_millis(2_000),
    );
    put(&mut ledger, on_time);
    let late = op_at(
        &ledger,
        &key(1),
        join(&ledger, 50, 61),
        Timestamp::from_millis(2_001),
    );
    put(&mut ledger, late);
    assert!(is_member(&ledger, 60));
    assert!(!is_member(&ledger, 61));
}

#[test]
fn revoke_wins_over_concurrent_join() {
    let mut base = network(&[], 1);
    act(&mut base, &key(1), create_invite(50, 5, 10_000));
    // A join the revoke has seen stays valid.
    let body = join(&base, 50, 60);
    act(&mut base, &key(1), body);
    let revoke = op(
        &base,
        &key(1),
        OpBody::RevokeInvite {
            invite_key: key(50).verifying_key(),
        },
    );
    let concurrent = op(&base, &key(1), join(&base, 50, 61));
    let mut ledger = base.clone();
    put(&mut ledger, concurrent);
    assert!(is_member(&ledger, 61));
    put(&mut ledger, revoke);
    assert!(is_member(&ledger, 60));
    assert!(!is_member(&ledger, 61));
    assert!(ledger.view().invites().is_empty());
    // Later joins and re-creating the invite do not help.
    act(&mut ledger, &key(1), create_invite(50, 5, 10_000));
    let body = join(&ledger, 50, 62);
    act(&mut ledger, &key(1), body);
    assert!(!is_member(&ledger, 62));
}

#[test]
fn max_uses_under_concurrency_admits_first_in_order() {
    let mut base = network(&[], 1);
    act(&mut base, &key(1), create_invite(50, 1, 10_000));
    let first = op(&base, &key(1), join(&base, 50, 60));
    let second = op(&base, &key(1), join(&base, 50, 61));
    let first_wins = first.id() < second.id();
    let mut ledger = base.clone();
    put(&mut ledger, second);
    put(&mut ledger, first);
    assert_eq!(is_member(&ledger, 60), first_wins);
    assert_eq!(is_member(&ledger, 61), !first_wins);
    assert!(ledger.view().invites().is_empty());
}

#[test]
fn join_proof_verifies_for_its_network_only() {
    let ledger = network(&[], 1);
    let proof = JoinProof::create(ledger.network_id(), &key(50), &key(60), "phone").unwrap();
    assert_eq!(proof.verify(ledger.network_id()), Ok(()));
    assert_eq!(
        proof.verify(NetworkId::from_bytes([0; 32])),
        Err(LedgerError::BadJoinProof)
    );
    let mut renamed = proof.clone();
    renamed.name = String::from("other");
    assert_eq!(
        renamed.verify(ledger.network_id()),
        Err(LedgerError::BadJoinProof)
    );
    let mut swapped = proof;
    swapped.device = id(61);
    assert_eq!(
        swapped.verify(ledger.network_id()),
        Err(LedgerError::BadJoinProof)
    );
    let long = "x".repeat(MAX_NAME_LEN + 1);
    assert_eq!(
        JoinProof::create(ledger.network_id(), &key(50), &key(60), &long).unwrap_err(),
        LedgerError::NameTooLong(MAX_NAME_LEN + 1)
    );
}

// Records.

#[test]
fn records_are_last_writer_wins() {
    let mut base = network(&[2], 1);
    act(&mut base, &key(1), set_record("dns/nas", Some(b"one")));
    act(&mut base, &key(1), set_record("dns/nas", Some(b"two")));
    assert_eq!(base.view().records().get("dns/nas").unwrap(), b"two");

    // Concurrent writes: the higher (lamport, OpId) wins.
    let a = op(&base, &key(1), set_record("dns/tv", Some(b"a")));
    let b = op(&base, &key(2), set_record("dns/tv", Some(b"b")));
    let expected: &[u8] = if a.id() > b.id() { b"a" } else { b"b" };
    let mut ledger = base.clone();
    put(&mut ledger, b);
    put(&mut ledger, a);
    assert_eq!(ledger.view().records().get("dns/tv").unwrap(), expected);

    // A later write beats concurrent ones regardless of id.
    act(&mut ledger, &key(2), set_record("dns/tv", Some(b"c")));
    assert_eq!(ledger.view().records().get("dns/tv").unwrap(), b"c");

    // None deletes.
    act(&mut ledger, &key(1), set_record("dns/nas", None));
    assert!(!ledger.view().records().contains_key("dns/nas"));
}

// Orphans.

#[test]
fn orphans_are_attached_when_parents_arrive() {
    let mut source = network(&[], 1);
    let first = op(&source, &key(1), add_device(5, "nas"));
    put(&mut source, first.clone());
    let second = op(&source, &key(1), add_device(6, "tv"));
    put(&mut source, second.clone());
    let third = op(&source, &key(1), add_device(7, "car"));

    let mut ledger = network(&[], 1);
    assert_eq!(
        ledger.insert(third.clone()),
        Ok(InsertOutcome::Orphaned {
            missing: vec![second.id()]
        })
    );
    // Missing ancestors are found through buffered orphans.
    assert_eq!(
        ledger.insert(second),
        Ok(InsertOutcome::Orphaned {
            missing: vec![first.id()]
        })
    );
    assert_eq!(
        ledger.insert(third.clone()),
        Ok(InsertOutcome::Orphaned {
            missing: vec![first.id()]
        })
    );
    assert_eq!(ledger.orphan_count(), 2);
    assert!(!is_member(&ledger, 7));
    put(&mut ledger, first);
    assert_eq!(ledger.orphan_count(), 0);
    assert!(is_member(&ledger, 5) && is_member(&ledger, 6) && is_member(&ledger, 7));
    assert_eq!(ledger.heads(), &[third.id()]);
}

#[test]
fn orphan_buffer_is_bounded() {
    let base = network(&[], 1);
    let mut ledger = base.clone();
    let mut source = base;
    let root = op(&source, &key(1), set_record("r", Some(b"0")));
    put(&mut source, root);
    let mut last = None;
    for round in 0..=MAX_ORPHANS {
        let value = round.to_le_bytes();
        let signed = op(&source, &key(1), set_record("r", Some(&value)));
        put(&mut source, signed.clone());
        last = Some(ledger.insert(signed));
    }
    assert_eq!(ledger.orphan_count(), MAX_ORPHANS);
    assert_eq!(last, Some(Err(LedgerError::OrphanBufferFull)));
}

// Structural rejections.

/// A well-formed unsigned op by key 1 on top of `ledger`'s heads.
fn draft(ledger: &Ledger, body: OpBody) -> UnsignedOp {
    ledger.draft(id(1), body, NOW)
}

#[test]
fn rejects_signature_over_different_content() {
    let ledger = network(&[], 1);
    let unsigned = draft(&ledger, add_device(5, "nas"));
    let good = unsigned.encode().unwrap();
    let signed_bytes = raw_with(&good, &[&key(1)]);
    let mut tampered = unsigned;
    tampered.body = add_device(5, "nat");
    let tampered_bytes = tampered.encode().unwrap();
    // Swap the content while keeping the old signature.
    let (_, (_, signatures)) =
        envelope::decode::<(Vec<u8>, Vec<(NodeId, Signature)>)>(&signed_bytes, SIGNED_OP_TYPE, 1)
            .unwrap();
    let forged = envelope::encode(SIGNED_OP_TYPE, 1, &(tampered_bytes, signatures)).unwrap();
    assert_eq!(decode_err(&forged), LedgerError::BadSignature(id(1)));
}

#[test]
fn rejects_author_not_signer() {
    let ledger = network(&[], 1);
    let unsigned = draft(&ledger, add_device(5, "nas"));
    assert_eq!(
        decode_err(&raw(&unsigned, &[&key(2)])),
        LedgerError::AuthorNotSigner
    );
    assert_eq!(
        unsigned.sign(&key(2)).unwrap_err(),
        LedgerError::AuthorNotSigner
    );
}

#[test]
fn rejects_signature_list_problems() {
    let ledger = network(&[], 1);
    let unsigned = draft(&ledger, add_device(5, "nas"));
    assert_eq!(decode_err(&raw(&unsigned, &[])), LedgerError::NoSignatures);
    let (low, high) = if id(1) < id(2) { (1, 2) } else { (2, 1) };
    let unsorted = raw(&unsigned, &[&key(high), &key(low)]);
    assert_eq!(decode_err(&unsorted), LedgerError::SignaturesUnsorted);
    let duplicate = raw(&unsigned, &[&key(1), &key(1)]);
    assert_eq!(decode_err(&duplicate), LedgerError::SignaturesUnsorted);
    // Thirty-three distinct signers, sorted.
    let keys: Vec<SigningKey> = (1..=33).map(key).collect();
    let mut refs: Vec<&SigningKey> = keys.iter().collect();
    refs.sort_by_key(|signer| signer.node_id());
    assert_eq!(
        decode_err(&raw(&unsigned, &refs)),
        LedgerError::TooManySignatures
    );
}

#[test]
fn signature_merge_is_order_independent_and_keeps_author() {
    let ledger = network(&[], 1);
    let base = op(&ledger, &key(1), add_device(5, "nas"));
    let mut left = base.clone();
    let mut right = base;
    for seed in 2..=25 {
        left.add_signature(&key(seed)).unwrap();
    }
    for seed in 20..=45 {
        right.add_signature(&key(seed)).unwrap();
    }
    let mut a = left.clone();
    assert!(a.merge_signatures(&right));
    let mut b = right;
    assert!(b.merge_signatures(&left));
    assert_eq!(a, b);
    assert_eq!(a.signatures().len(), 32);
    assert!(a.signers().any(|signer| signer == id(1)));
}

#[test]
fn rejects_wrong_network_and_second_genesis() {
    let mut ledger = network(&[], 1);
    let other = network(&[2], 1);
    let foreign = op(&other, &key(1), add_device(5, "nas"));
    assert_eq!(ledger.insert(foreign), Err(LedgerError::NetworkMismatch));
    let other_genesis = other.get(&other.genesis_id()).cloned().unwrap();
    assert_eq!(
        ledger.insert(other_genesis),
        Err(LedgerError::UnexpectedGenesis)
    );
}

#[test]
fn rejects_wrong_lamport() {
    let mut ledger = network(&[], 1);
    let mut unsigned = draft(&ledger, add_device(5, "nas"));
    unsigned.lamport = 2;
    let signed = unsigned.sign(&key(1)).unwrap();
    assert_eq!(
        ledger.insert(signed),
        Err(LedgerError::WrongLamport {
            expected: 1,
            found: 2
        })
    );
    unsigned.lamport = 0;
    assert_eq!(
        unsigned.encode().unwrap_err(),
        LedgerError::WrongLamport {
            expected: 1,
            found: 0
        }
    );
    assert_eq!(ledger.len(), 1);
}

#[test]
fn rejects_parent_problems() {
    let ledger = network(&[], 1);
    let genesis = ledger.genesis_id();
    let mut unsigned = draft(&ledger, add_device(5, "nas"));

    unsigned.parents = Vec::new();
    assert_eq!(
        decode_err(&raw(&unsigned, &[&key(1)])),
        LedgerError::NoParents
    );

    unsigned.parents = vec![genesis, genesis];
    assert_eq!(
        decode_err(&raw(&unsigned, &[&key(1)])),
        LedgerError::ParentsUnsorted
    );

    let low = OpId::from_bytes([0; 32]);
    unsigned.parents = vec![genesis, low];
    if genesis > low {
        assert_eq!(
            decode_err(&raw(&unsigned, &[&key(1)])),
            LedgerError::ParentsUnsorted
        );
    }

    unsigned.parents = (0..=16_u8)
        .map(|byte| OpId::from_bytes([byte; 32]))
        .collect();
    assert_eq!(
        decode_err(&raw(&unsigned, &[&key(1)])),
        LedgerError::TooManyParents(17)
    );

    unsigned.parents = vec![genesis];
    unsigned.network = None;
    assert_eq!(
        decode_err(&raw(&unsigned, &[&key(1)])),
        LedgerError::MissingNetwork
    );
}

#[test]
fn rejects_malformed_genesis() {
    let ledger = network(&[], 1);
    let genesis = ledger.get(&ledger.genesis_id()).unwrap().op().clone();
    let mut with_parent = genesis.clone();
    with_parent.parents = vec![ledger.genesis_id()];
    assert_eq!(
        decode_err(&raw(&with_parent, &[&key(1)])),
        LedgerError::MalformedGenesis
    );
    let mut with_network = genesis.clone();
    with_network.network = Some(ledger.network_id());
    assert_eq!(
        decode_err(&raw(&with_network, &[&key(1)])),
        LedgerError::MalformedGenesis
    );

    let with_admins = |admins: Vec<NodeId>| {
        let mut op = genesis.clone();
        if let OpBody::Genesis { admins: slot, .. } = &mut op.body {
            *slot = admins;
        }
        decode_err(&raw(&op, &[&key(1)]))
    };
    assert_eq!(with_admins(Vec::new()), LedgerError::BadAdmins);
    assert_eq!(with_admins(vec![id(2)]), LedgerError::BadAdmins);
    assert_eq!(with_admins(vec![id(1), id(1)]), LedgerError::BadAdmins);
    let mut descending = vec![id(1), id(2)];
    descending.sort();
    descending.reverse();
    assert_eq!(with_admins(descending), LedgerError::BadAdmins);
    let bad_key = invalid_key();
    let mut with_bad = vec![id(1), bad_key];
    with_bad.sort();
    assert_eq!(with_admins(with_bad), LedgerError::InvalidKey(bad_key));
}

#[test]
fn rejects_field_limits() {
    let ledger = network(&[], 1);
    let check = |body: OpBody| draft(&ledger, body).encode().unwrap_err();
    let reject = |body: OpBody| decode_err(&raw(&draft(&ledger, body), &[&key(1)]));

    let long_name = "x".repeat(MAX_NAME_LEN + 1);
    assert_eq!(
        check(add_device(5, &long_name)),
        LedgerError::NameTooLong(65)
    );
    assert_eq!(
        reject(add_device(5, &long_name)),
        LedgerError::NameTooLong(65)
    );

    let tags = |tags: Vec<&str>| OpBody::AddDevice {
        device: id(5),
        name: String::from("nas"),
        tags: tags.into_iter().map(String::from).collect(),
    };
    let many: Vec<String> = (0..=MAX_TAGS)
        .map(|index| std::format!("t{index:02}"))
        .collect();
    assert_eq!(
        reject(tags(many.iter().map(String::as_str).collect())),
        LedgerError::TooManyTags(17)
    );
    assert_eq!(reject(tags(vec![""])), LedgerError::BadTagLength(0));
    let long_tag = "t".repeat(33);
    assert_eq!(reject(tags(vec![&long_tag])), LedgerError::BadTagLength(33));
    assert_eq!(reject(tags(vec!["b", "a"])), LedgerError::TagsUnsorted);
    assert_eq!(reject(tags(vec!["a", "a"])), LedgerError::TagsUnsorted);

    let long_key = "k".repeat(MAX_RECORD_KEY_LEN + 1);
    assert_eq!(
        reject(set_record(&long_key, None)),
        LedgerError::BadRecordKeyLength(129)
    );
    assert_eq!(
        reject(set_record("", None)),
        LedgerError::BadRecordKeyLength(0)
    );
    let big = vec![0_u8; MAX_RECORD_VALUE_LEN + 1];
    assert_eq!(
        reject(set_record("k", Some(&big))),
        LedgerError::RecordValueTooLarge(4097)
    );
    let max = vec![0_u8; MAX_RECORD_VALUE_LEN];
    assert!(draft(&ledger, set_record("k", Some(&max))).encode().is_ok());

    assert_eq!(reject(create_invite(50, 0, 1)), LedgerError::ZeroMaxUses);
    assert_eq!(
        reject(OpBody::SetThreshold { threshold: 0 }),
        LedgerError::BadThreshold(0)
    );
    let bad_key = invalid_key();
    assert_eq!(
        reject(OpBody::AddAdmin { admin: bad_key }),
        LedgerError::InvalidKey(bad_key)
    );
}

/// An identity that is not a valid Ed25519 public key.
fn invalid_key() -> NodeId {
    (0..=u8::MAX)
        .map(|byte| NodeId::from_bytes([byte; 32]))
        .find(|candidate| kith_crypto::VerifyingKey::from_node_id(*candidate).is_err())
        .unwrap()
}

#[test]
fn rejects_bad_join_proof() {
    let ledger = network(&[], 1);
    let mut proof = JoinProof::create(ledger.network_id(), &key(50), &key(60), "phone").unwrap();
    proof.name = String::from("evil");
    let unsigned = draft(&ledger, OpBody::Join(proof));
    assert_eq!(
        decode_err(&raw(&unsigned, &[&key(1)])),
        LedgerError::BadJoinProof
    );
}

#[test]
fn rejects_oversized_trailing_and_mistyped_envelopes() {
    let huge = vec![0_u8; crate::MAX_SIGNED_OP_LEN + 1];
    assert!(matches!(decode_err(&huge), LedgerError::TooLarge { .. }));
    let ledger = network(&[], 1);
    let mut bytes = op(&ledger, &key(1), add_device(5, "nas")).to_bytes();
    bytes.push(0);
    assert!(matches!(decode_err(&bytes), LedgerError::Wire(_)));
    // An unsigned op in place of a signed one.
    let unsigned = draft(&ledger, add_device(5, "nas")).encode().unwrap();
    assert!(matches!(decode_err(&unsigned), LedgerError::Wire(_)));
    // Unsigned bytes over the op limit.
    let padded = vec![0_u8; crate::MAX_OP_LEN + 1];
    let wrapped = envelope::encode(
        SIGNED_OP_TYPE,
        1,
        &(padded, Vec::<(NodeId, Signature)>::new()),
    )
    .unwrap();
    assert!(matches!(decode_err(&wrapped), LedgerError::TooLarge { .. }));
}

// Wire format.

#[test]
fn golden_bytes_of_a_set_record_op() {
    // Pins the format: changing these bytes is a protocol break.
    let op = UnsignedOp {
        network: Some(NetworkId::from_bytes([1; 32])),
        parents: vec![OpId::from_bytes([2; 32])],
        author: NodeId::from_bytes([3; 32]),
        lamport: 1,
        timestamp: Timestamp::from_millis(300),
        body: OpBody::SetRecord {
            key: String::from("a"),
            value: Some(vec![7]),
        },
    };
    let mut expected = vec![b'k', b't', 0x10, 1, 1];
    expected.extend_from_slice(&[1; 32]);
    expected.push(1);
    expected.extend_from_slice(&[2; 32]);
    expected.extend_from_slice(&[3; 32]);
    // lamport 1, timestamp 300 as a varint, variant 8, key "a", Some([7]).
    expected.extend_from_slice(&[1, 0xac, 0x02, 8, 1, b'a', 1, 1, 7]);
    assert_eq!(op.encode().unwrap(), expected);
}

// Properties.

/// A device key for convergence-test target `target`: the four writers, then others.
fn device_seed(target: u8) -> u8 {
    if target < 4 { target + 1 } else { target + 10 }
}

fn random_body(replica: &Ledger, action: u8, target: u8) -> Option<OpBody> {
    let invite = 30 + target % 2;
    Some(match action {
        0 => add_device(device_seed(target), "d"),
        1 => OpBody::RemoveDevice {
            device: id(device_seed(target)),
        },
        2 => set_record(
            ["a", "b", "c"][usize::from(target % 3)],
            (!target.is_multiple_of(4)).then_some(&[target][..]),
        ),
        3 => OpBody::AddAdmin {
            admin: id(1 + target % 4),
        },
        4 => OpBody::RemoveAdmin {
            admin: id(1 + target % 4),
        },
        5 => create_invite(invite, 1 + u16::from(target % 2), 1_000),
        6 => join(replica, invite, 40 + target),
        7 => OpBody::RevokeInvite {
            invite_key: key(invite).verifying_key(),
        },
        8 => OpBody::SetThreshold {
            threshold: 1 + target % 2,
        },
        _ => return None,
    })
}

/// Deterministic Fisher–Yates shuffle driven by xorshift.
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn replicas_converge_in_any_order(
        steps in proptest::collection::vec((0_usize..4, 0_u8..11, 0_u8..8, 0_usize..4), 1..40),
        seeds in proptest::collection::vec(any::<u64>(), 3),
    ) {
        let base = network(&[2], 1);
        let mut replicas = vec![base.clone(); 4];
        for (writer, action, target, other) in steps {
            if action >= 9 {
                // Gossip: a writer learns everything another writer knows.
                let source = replicas[other].clone();
                sync(&mut replicas[writer], &source);
                continue;
            }
            let author = key(u8::try_from(writer).unwrap() + 1);
            let replica = &mut replicas[writer];
            let Some(body) = random_body(replica, action, target) else { continue };
            // Joins carry a timestamp so some are past the invite's expiry.
            let timestamp = Timestamp::from_millis(u64::from(target) * 200);
            let signed = op_at(replica, &author, body, timestamp);
            prop_assert_eq!(replica.insert(signed), Ok(InsertOutcome::Inserted));
        }

        let mut all: BTreeMap<OpId, SignedOp> = BTreeMap::new();
        for replica in &replicas {
            for id in replica.op_ids() {
                all.insert(*id, replica.get(id).unwrap().clone());
            }
        }
        let mut reference = base.clone();
        for replica in &replicas {
            sync(&mut reference, replica);
        }
        prop_assert_eq!(reference.len(), all.len());

        for seed in seeds {
            let mut ops: Vec<SignedOp> = all.values().cloned().collect();
            shuffle(&mut ops, seed);
            let mut fresh = base.clone();
            let mut one_by_one = Vec::new();
            for op in ops.clone() {
                one_by_one.push(fresh.insert(op));
            }
            prop_assert!(one_by_one.iter().all(Result::is_ok));
            prop_assert_eq!(fresh.orphan_count(), 0);
            prop_assert_eq!(fresh.heads(), reference.heads());
            prop_assert_eq!(fresh.view(), reference.view());

            // A batch of the same ops in the same order: same per-op outcomes, same view.
            let mut batched = base.clone();
            let outcome = batched.insert_batch(ops);
            prop_assert_eq!(outcome.results, one_by_one);
            prop_assert_eq!(batched.view(), reference.view());
        }
    }

    #[test]
    fn decoding_arbitrary_or_mutated_bytes_never_panics(
        noise in proptest::collection::vec(any::<u8>(), 0..512),
        position in any::<usize>(),
        byte in any::<u8>(),
        cut in any::<bool>(),
    ) {
        let _ignored = SignedOp::from_bytes(&noise);
        let ledger = network(&[], 1);
        let mut bytes = op(&ledger, &key(1), join(&ledger, 50, 60)).to_bytes();
        let position = position % bytes.len();
        if cut {
            bytes.truncate(position);
        } else {
            bytes[position] = byte;
        }
        let _ignored = SignedOp::from_bytes(&bytes);
    }
}
