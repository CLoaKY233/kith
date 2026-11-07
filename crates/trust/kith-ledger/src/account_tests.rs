//! Tests for people in the ledger: passkey approvals, roles, consoles and recovery.

extern crate std;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use kith_crypto::SigningKey;
use kith_crypto::webauthn::{encode_challenge, sha256};
use kith_types::{AccountId, NodeId, OpId, Timestamp};
use p256::ecdsa::signature::Signer as _;

use crate::{
    AccountChange, Approval, Assertion, Credential, InsertOutcome, Ledger, NetworkParams, OpBody,
    Role, SignedOp,
};

const NOW: Timestamp = Timestamp::from_millis(1_000);
const RP: &str = "vpn.example.com";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_seed(&[seed; 32])
}

fn id(seed: u8) -> NodeId {
    key(seed).node_id()
}

/// A network founded by admin device 1.
fn network() -> Ledger {
    let params = NetworkParams {
        ipv4: "100.64.0.0/10".parse().unwrap(),
        ipv6: "fd7a:115c:a1e0::/48".parse().unwrap(),
    };
    Ledger::create(&key(1), "home", "laptop", params, &[], 1, NOW).unwrap()
}

/// A person's passkey: a P-256 key signing WebAuthn assertions as a browser would.
struct Passkey {
    secret: p256::ecdsa::SigningKey,
    credential: Credential,
}

impl Passkey {
    fn new(seed: u8) -> Self {
        let secret = p256::ecdsa::SigningKey::from_slice(&[seed; 32]).unwrap();
        let public_key = secret.verifying_key().to_sec1_bytes().to_vec();
        Self {
            secret,
            credential: Credential {
                id: alloc::vec![seed; 16],
                public_key,
                rp_id: String::from(RP),
            },
        }
    }

    fn account(&self) -> AccountId {
        self.credential.account_id()
    }

    fn add(&self, name: &str, role: Role) -> OpBody {
        OpBody::AddAccount {
            name: String::from(name),
            role,
            credential: self.credential.clone(),
        }
    }

    /// A good assertion of `challenge`.
    fn approve(&self, challenge: &OpId) -> Approval {
        self.assertion(challenge.as_bytes(), "webauthn.get", RP, 0x05)
    }

    fn assertion(&self, challenge: &[u8], kind: &str, rp: &str, flags: u8) -> Approval {
        let mut authenticator_data = sha256(rp.as_bytes()).to_vec();
        authenticator_data.extend([flags, 0, 0, 0, 9]);
        let client_data_json = format!(
            r#"{{"type":"{kind}","challenge":"{}","origin":"https://{rp}"}}"#,
            encode_challenge(challenge)
        )
        .into_bytes();
        let mut message = authenticator_data.clone();
        message.extend(sha256(&client_data_json));
        let signature: p256::ecdsa::DerSignature = self.secret.sign(&message);
        Approval::Passkey(Assertion {
            account: self.account(),
            credential_id: self.credential.id.clone(),
            authenticator_data,
            client_data_json,
            signature: signature.as_bytes().to_vec(),
        })
    }
}

/// `change` drafted by device `author` on `ledger`'s heads, approved over its challenge.
fn approved(
    ledger: &Ledger,
    author: &SigningKey,
    change: OpBody,
    approve: impl FnOnce(&OpId) -> Approval,
) -> SignedOp {
    let plain = ledger.draft(author.node_id(), change, NOW);
    let challenge = plain.id().unwrap();
    let signed = plain
        .approve(approve(&challenge))
        .unwrap()
        .sign(author)
        .unwrap();
    assert_eq!(signed.challenge(), Some(challenge));
    signed
}

fn signed(ledger: &Ledger, author: &SigningKey, body: OpBody) -> SignedOp {
    ledger
        .draft(author.node_id(), body, NOW)
        .sign(author)
        .unwrap()
}

/// `body` made by `author` for `actor`.
fn acted(ledger: &Ledger, author: &SigningKey, actor: AccountId, body: OpBody) -> SignedOp {
    ledger
        .draft(author.node_id(), body, NOW)
        .act_for(actor)
        .unwrap()
        .sign(author)
        .unwrap()
}

fn put(ledger: &mut Ledger, op: SignedOp) -> OpId {
    let id = op.id();
    assert_eq!(ledger.insert(op), Ok(InsertOutcome::Inserted));
    id
}

/// Whether `op` took effect on the account chain.
fn applied(ledger: &Ledger, op: OpId) -> bool {
    ledger.view().account_changes().contains(&op)
}

fn role(ledger: &Ledger, passkey: &Passkey) -> Option<Role> {
    ledger
        .view()
        .account(&passkey.account())
        .map(|account| account.role)
}

fn rename(seed: u8, name: &str) -> OpBody {
    OpBody::RenameDevice {
        device: id(seed),
        name: String::from(name),
    }
}

fn name_of(ledger: &Ledger, seed: u8) -> Option<String> {
    ledger
        .view()
        .devices()
        .get(&id(seed))
        .map(|device| device.name.clone())
}

/// A network whose owner `passkey` was added by the founding device.
fn owned_by(passkey: &Passkey) -> Ledger {
    let mut ledger = network();
    let first = signed(&ledger, &key(1), passkey.add("ann", Role::Owner));
    put(&mut ledger, first);
    ledger
}

#[test]
fn the_admin_device_adds_only_the_first_owner() {
    let mut ledger = network();
    let view = ledger.view();
    assert!(view.accounts().is_empty() && view.consoles().is_empty());
    assert!(view.recovery_key().is_none() && view.account_changes().is_empty());

    // Not as an admin, and not by a device that is not an admin.
    let ann = Passkey::new(1);
    let as_admin = signed(&ledger, &key(1), ann.add("ann", Role::Admin));
    let stranger = signed(&ledger, &key(3), ann.add("ann", Role::Owner));
    put(&mut ledger, as_admin);
    put(&mut ledger, stranger);
    assert!(ledger.view().accounts().is_empty());

    let first = signed(&ledger, &key(1), ann.add("ann", Role::Owner));
    let first = put(&mut ledger, first);
    assert!(applied(&ledger, first));
    let account = ledger.view().account(&ann.account()).unwrap();
    assert_eq!((account.name.as_str(), account.role), ("ann", Role::Owner));
    assert_eq!(account.credentials, core::slice::from_ref(&ann.credential));

    // From now on the device path is closed, for people and for the recovery key.
    let bob = Passkey::new(2);
    let second = signed(&ledger, &key(1), bob.add("bob", Role::Owner));
    let recovery = signed(
        &ledger,
        &key(1),
        OpBody::SetRecoveryKey {
            key: key(40).verifying_key(),
        },
    );
    put(&mut ledger, second);
    put(&mut ledger, recovery);
    assert_eq!(ledger.view().accounts().len(), 1);
    assert!(ledger.view().recovery_key().is_none());
}

#[test]
fn passkey_approvals_must_sign_this_op_for_this_site() {
    let ann = Passkey::new(1);
    let mut ledger = owned_by(&ann);
    let console = OpBody::AuthorizeConsole { device: id(9) };

    // Signed by a different key claiming ann's passkey.
    let mallory = Passkey {
        credential: ann.credential.clone(),
        ..Passkey::new(66)
    };
    let forged = approved(&ledger, &key(1), console.clone(), |c| mallory.approve(c));
    // A real signature, but over another op.
    let other = OpId::from_bytes([7; 32]);
    let replayed = approved(&ledger, &key(1), console.clone(), |_| ann.approve(&other));
    // For another site, as a creation, or without the user verified.
    let phished = approved(&ledger, &key(1), console.clone(), |c| {
        ann.assertion(c.as_bytes(), "webauthn.get", "evil.example", 0x05)
    });
    let created = approved(&ledger, &key(1), console.clone(), |c| {
        ann.assertion(c.as_bytes(), "webauthn.create", RP, 0x05)
    });
    let unverified = approved(&ledger, &key(1), console.clone(), |c| {
        ann.assertion(c.as_bytes(), "webauthn.get", RP, 0x01)
    });
    for bad in [forged, replayed, phished, created, unverified] {
        let op = put(&mut ledger, bad);
        assert!(!applied(&ledger, op));
    }
    assert!(ledger.view().consoles().is_empty());

    let good = approved(&ledger, &key(1), console, |c| ann.approve(c));
    // It travels like any op and every replica agrees.
    let decoded = SignedOp::from_bytes(&good.to_bytes()).unwrap();
    assert_eq!(decoded, good);
    let op = put(&mut ledger, decoded);
    assert!(applied(&ledger, op));
    assert!(ledger.view().is_console(&id(9)));
}

#[test]
fn roles_decide_who_changes_people() {
    let ann = Passkey::new(1);
    let (ada, max, oz) = (Passkey::new(2), Passkey::new(3), Passkey::new(4));
    let mut ledger = owned_by(&ann);
    let add_ada = approved(&ledger, &key(1), ada.add("ada", Role::Admin), |c| {
        ann.approve(c)
    });
    put(&mut ledger, add_ada);
    // Admins add members, not owners.
    let add_max = approved(&ledger, &key(1), max.add("max", Role::Member), |c| {
        ada.approve(c)
    });
    let add_oz = approved(&ledger, &key(1), oz.add("oz", Role::Owner), |c| {
        ada.approve(c)
    });
    put(&mut ledger, add_max);
    put(&mut ledger, add_oz);
    assert_eq!(role(&ledger, &max), Some(Role::Member));
    assert_eq!(role(&ledger, &oz), None);

    // Only owners set roles; members change nothing about people.
    let promote = |account: &Passkey| OpBody::SetRole {
        account: account.account(),
        role: Role::Admin,
    };
    let by_admin = approved(&ledger, &key(1), promote(&max), |c| ada.approve(c));
    let by_member = approved(&ledger, &key(1), oz.add("oz", Role::Member), |c| {
        max.approve(c)
    });
    put(&mut ledger, by_admin);
    put(&mut ledger, by_member);
    assert_eq!(role(&ledger, &max), Some(Role::Member));
    assert_eq!(role(&ledger, &oz), None);
    let by_owner = approved(&ledger, &key(1), promote(&max), |c| ann.approve(c));
    put(&mut ledger, by_owner);
    assert_eq!(role(&ledger, &max), Some(Role::Admin));

    // The last owner can be neither demoted nor removed, even by themselves.
    let demote = approved(&ledger, &key(1), promote(&ann), |c| ann.approve(c));
    let remove = approved(
        &ledger,
        &key(1),
        OpBody::RemoveAccount {
            account: ann.account(),
        },
        |c| ann.approve(c),
    );
    put(&mut ledger, demote);
    put(&mut ledger, remove);
    assert_eq!(role(&ledger, &ann), Some(Role::Owner));
}

#[test]
fn removing_a_person_ends_their_approvals_actions_and_devices() {
    let ann = Passkey::new(1);
    let ada = Passkey::new(2);
    let mut ledger = owned_by(&ann);
    for change in [
        ada.add("ada", Role::Admin),
        OpBody::AuthorizeConsole { device: id(9) },
    ] {
        let op = approved(&ledger, &key(1), change, |c| ann.approve(c));
        put(&mut ledger, op);
    }
    let add = signed(
        &ledger,
        &key(1),
        OpBody::AddDevice {
            device: id(5),
            name: String::from("phone"),
            tags: Vec::new(),
        },
    );
    put(&mut ledger, add);
    let assign = acted(
        &ledger,
        &key(9),
        ada.account(),
        OpBody::AssignDevice {
            device: id(5),
            account: ada.account(),
        },
    );
    put(&mut ledger, assign);
    assert_eq!(
        ledger.view().devices_of(&ada.account()).collect::<Vec<_>>(),
        [id(5)]
    );

    let removal = approved(
        &ledger,
        &key(1),
        OpBody::RemoveAccount {
            account: ada.account(),
        },
        |c| ann.approve(c),
    );
    put(&mut ledger, removal);
    assert_eq!(role(&ledger, &ada), None);
    assert!(!ledger.view().is_member(&id(5)));
    assert!(ledger.view().device_owners().is_empty());

    // Afterwards her passkey approves nothing and the console cannot act for her.
    let late = approved(
        &ledger,
        &key(1),
        Passkey::new(3).add("max", Role::Member),
        |c| ada.approve(c),
    );
    let late = put(&mut ledger, late);
    assert!(!applied(&ledger, late));
    let rename_op = acted(&ledger, &key(9), ada.account(), rename(1, "desk"));
    put(&mut ledger, rename_op);
    assert_eq!(name_of(&ledger, 1).as_deref(), Some("laptop"));
}

#[test]
fn consoles_act_only_for_admins_and_lose_to_a_concurrent_revoke() {
    let ann = Passkey::new(1);
    let max = Passkey::new(3);
    let mut ledger = owned_by(&ann);
    for change in [
        max.add("max", Role::Member),
        OpBody::AuthorizeConsole { device: id(9) },
    ] {
        let op = approved(&ledger, &key(1), change, |c| ann.approve(c));
        put(&mut ledger, op);
    }

    // Without an actor, for a member, or from a device that is no console: refused.
    let plain = signed(&ledger, &key(9), rename(1, "plain"));
    let for_member = acted(&ledger, &key(9), max.account(), rename(1, "member"));
    let not_console = acted(&ledger, &key(8), ann.account(), rename(1, "stranger"));
    for op in [plain, for_member, not_console] {
        put(&mut ledger, op);
    }
    assert_eq!(name_of(&ledger, 1).as_deref(), Some("laptop"));

    // For an owner: applied, and the admin device may also name who asked.
    let for_owner = acted(&ledger, &key(9), ann.account(), rename(1, "desk"));
    put(&mut ledger, for_owner);
    assert_eq!(name_of(&ledger, 1).as_deref(), Some("desk"));
    let by_device = acted(&ledger, &key(1), ann.account(), rename(1, "desk-2"));
    put(&mut ledger, by_device);
    assert_eq!(name_of(&ledger, 1).as_deref(), Some("desk-2"));

    // A rename concurrent with revoking the console loses.
    let base = ledger.clone();
    let revoke = approved(
        &base,
        &key(1),
        OpBody::RevokeConsole { device: id(9) },
        |c| ann.approve(c),
    );
    let concurrent = acted(&base, &key(9), ann.account(), rename(1, "late"));
    put(&mut ledger, concurrent);
    assert_eq!(name_of(&ledger, 1).as_deref(), Some("late"));
    put(&mut ledger, revoke);
    assert!(!ledger.view().is_console(&id(9)));
    assert_eq!(name_of(&ledger, 1).as_deref(), Some("desk-2"));
}

#[test]
fn a_console_makes_a_member_only_a_single_use_untagged_invite() {
    let ann = Passkey::new(1);
    let max = Passkey::new(3);
    let mut ledger = owned_by(&ann);
    for change in [
        max.add("max", Role::Member),
        OpBody::AuthorizeConsole { device: id(9) },
    ] {
        let op = approved(&ledger, &key(1), change, |c| ann.approve(c));
        put(&mut ledger, op);
    }
    let invite = |seed: u8, max_uses: u16, tags: Vec<String>| OpBody::CreateInvite {
        invite_key: key(seed).verifying_key(),
        expires: Timestamp::from_millis(600_000),
        max_uses,
        tags,
    };

    let reusable = acted(&ledger, &key(9), max.account(), invite(20, 5, Vec::new()));
    let tagged = acted(
        &ledger,
        &key(9),
        max.account(),
        invite(21, 1, alloc::vec!["admin".into()]),
    );
    let not_console = acted(&ledger, &key(8), max.account(), invite(22, 1, Vec::new()));
    let stranger = acted(
        &ledger,
        &key(9),
        Passkey::new(5).account(),
        invite(23, 1, Vec::new()),
    );
    for op in [reusable, tagged, not_console, stranger] {
        put(&mut ledger, op);
    }
    assert!(ledger.view().invites().is_empty());

    let own = acted(&ledger, &key(9), max.account(), invite(24, 1, Vec::new()));
    put(&mut ledger, own);
    assert!(
        ledger
            .view()
            .invites()
            .contains_key(&key(24).verifying_key().to_bytes())
    );
    assert_eq!(ledger.view().invites().len(), 1);
}

#[test]
fn a_person_sets_only_their_own_devices_settings_through_a_console() {
    let ann = Passkey::new(1);
    let max = Passkey::new(3);
    let oz = Passkey::new(4);
    let mut ledger = owned_by(&ann);
    for change in [
        max.add("max", Role::Member),
        oz.add("oz", Role::Member),
        OpBody::AuthorizeConsole { device: id(9) },
    ] {
        let op = approved(&ledger, &key(1), change, |c| ann.approve(c));
        put(&mut ledger, op);
    }
    for (seed, owner) in [(5, &max), (6, &oz)] {
        let add = signed(
            &ledger,
            &key(1),
            OpBody::AddDevice {
                device: id(seed),
                name: format!("phone{seed}"),
                tags: Vec::new(),
            },
        );
        put(&mut ledger, add);
        let assign = acted(
            &ledger,
            &key(9),
            ann.account(),
            OpBody::AssignDevice {
                device: id(seed),
                account: owner.account(),
            },
        );
        put(&mut ledger, assign);
    }
    let settings = |seed: u8| OpBody::SetRecord {
        key: format!("device/{}", id(seed)),
        value: Some(alloc::vec![seed]),
    };

    let others = acted(&ledger, &key(9), max.account(), settings(6));
    let not_console = acted(&ledger, &key(8), max.account(), settings(5));
    let by_device = signed(&ledger, &key(5), settings(5));
    for op in [others, not_console, by_device] {
        put(&mut ledger, op);
    }
    assert!(ledger.view().records().is_empty());

    let own = acted(&ledger, &key(9), max.account(), settings(5));
    put(&mut ledger, own);
    let records = ledger.view().records();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records.get(&format!("device/{}", id(5))),
        Some(&alloc::vec![5])
    );
}

#[test]
fn the_recovery_key_adds_people_and_consoles_like_an_owner() {
    let mut ledger = network();
    let recovery = key(40);
    let set = signed(
        &ledger,
        &key(1),
        OpBody::SetRecoveryKey {
            key: recovery.verifying_key(),
        },
    );
    put(&mut ledger, set);
    assert_eq!(
        ledger.view().recovery_key(),
        Some(&recovery.verifying_key())
    );
    let ann = Passkey::new(1);
    let first = signed(&ledger, &key(1), ann.add("ann", Role::Owner));
    put(&mut ledger, first);

    // A lost owner is replaced, and a new console authorized, with the recovery words.
    let bob = Passkey::new(2);
    let wrong = approved(&ledger, &key(1), bob.add("bob", Role::Owner), |c| {
        Approval::recovery(&key(41), c)
    });
    put(&mut ledger, wrong);
    assert_eq!(role(&ledger, &bob), None);
    for change in [
        bob.add("bob", Role::Owner),
        OpBody::AuthorizeConsole { device: id(9) },
    ] {
        let op = approved(&ledger, &key(1), change, |c| {
            Approval::recovery(&recovery, c)
        });
        let op = put(&mut ledger, op);
        assert!(applied(&ledger, op));
    }
    assert_eq!(role(&ledger, &bob), Some(Role::Owner));
    assert!(ledger.view().is_console(&id(9)));

    // It does nothing else.
    let remove = approved(
        &ledger,
        &key(1),
        OpBody::RemoveAccount {
            account: ann.account(),
        },
        |c| Approval::recovery(&recovery, c),
    );
    put(&mut ledger, remove);
    assert_eq!(role(&ledger, &ann), Some(Role::Owner));
}

#[test]
fn only_account_changes_take_approvals_and_only_admin_changes_take_actors() {
    let ledger = network();
    let draft = ledger.draft(id(1), rename(1, "x"), NOW);
    let approval = Passkey::new(1).approve(&OpId::from_bytes([0; 32]));
    assert!(draft.clone().approve(approval).is_err());
    let account = Passkey::new(1).account();
    let people = ledger.draft(id(1), OpBody::RemoveAccount { account }, NOW);
    assert!(people.act_for(account).is_err());
    assert!(draft.act_for(account).is_ok());
    assert_eq!(
        AccountChange::from_body(OpBody::RemoveAccount { account }).map(|c| c.to_body()),
        Some(OpBody::RemoveAccount { account })
    );
}

#[test]
fn a_person_or_an_owner_adds_another_passkey() {
    let ann = Passkey::new(1);
    let bob = Passkey::new(2);
    let carol = Passkey::new(3);
    let mut ledger = owned_by(&ann);
    for person in [&bob, &carol] {
        let add = approved(&ledger, &key(1), person.add("p", Role::Member), |c| {
            ann.approve(c)
        });
        put(&mut ledger, add);
    }
    let spare = Passkey::new(4).credential;
    let add_spare = |account: AccountId| OpBody::AddCredential {
        account,
        credential: spare.clone(),
    };

    let by_carol = approved(&ledger, &key(1), add_spare(bob.account()), |c| {
        carol.approve(c)
    });
    let by_carol = put(&mut ledger, by_carol);
    assert!(!applied(&ledger, by_carol));

    let by_bob = approved(&ledger, &key(1), add_spare(bob.account()), |c| {
        bob.approve(c)
    });
    let by_bob = put(&mut ledger, by_bob);
    assert!(applied(&ledger, by_bob));
    let credentials = &ledger.view().account(&bob.account()).unwrap().credentials;
    assert_eq!(credentials.len(), 2);

    let again = approved(&ledger, &key(1), add_spare(bob.account()), |c| {
        ann.approve(c)
    });
    let again = put(&mut ledger, again);
    assert!(!applied(&ledger, again));
}

#[test]
fn a_console_admin_device_acts_only_for_people_until_revoked() {
    let ann = Passkey::new(1);
    let mut ledger = owned_by(&ann);
    let authorize = approved(
        &ledger,
        &key(1),
        OpBody::AuthorizeConsole { device: id(1) },
        |c| ann.approve(c),
    );
    put(&mut ledger, authorize);

    let plain = signed(&ledger, &key(1), rename(1, "plain"));
    put(&mut ledger, plain);
    assert_ne!(name_of(&ledger, 1).as_deref(), Some("plain"));
    let for_ann = acted(&ledger, &key(1), ann.account(), rename(1, "for-ann"));
    put(&mut ledger, for_ann);
    assert_eq!(name_of(&ledger, 1).as_deref(), Some("for-ann"));

    let revoke = approved(
        &ledger,
        &key(1),
        OpBody::RevokeConsole { device: id(1) },
        |c| ann.approve(c),
    );
    put(&mut ledger, revoke);
    let after = acted(&ledger, &key(1), ann.account(), rename(1, "after"));
    put(&mut ledger, after);
    assert_eq!(name_of(&ledger, 1).as_deref(), Some("for-ann"));
}
