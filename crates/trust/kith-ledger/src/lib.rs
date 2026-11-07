//! The signed, causally ordered membership ledger and the network view derived from it.
//!
//! Sans-IO: no I/O, no clock, no runtime. Every device keeps a full replica of a signed,
//! append-only DAG of ops and derives the network state from it. Any two replicas holding
//! the same ops derive the same [`NetworkView`], whatever the insertion order.
//!
//! # Ops
//!
//! An op names its network, its parents (1 to [`MAX_PARENTS`], sorted), its author, a
//! lamport clock exactly one more than the largest parent's, a timestamp, and an
//! [`OpBody`]. Genesis has no network, no parents and lamport 0; its
//! [`OpId`](kith_types::OpId) *is* the [`NetworkId`](kith_types::NetworkId). A
//! [`SignedOp`] keeps the exact bytes it was decoded from and never re-encodes; signatures
//! sit outside the hashed bytes, so admins can co-sign and replicas merge co-signatures.
//!
//! # Validity
//!
//! "X's causal past" means X's strict ancestors. Validity has two stages, so no rule
//! depends on itself.
//!
//! **Stage A, the authority chain.** Genesis, `AddAdmin`, `RemoveAdmin` and `SetThreshold`
//! form a linear chain from genesis. Candidates for the link after head H are authority
//! ops descending from H that are authored by an admin of the state at H, signed by at
//! least its threshold of distinct admins, and make an allowed change (no duplicate admin,
//! no removal below the threshold, threshold between 1 and the admin count). Of the
//! earliest candidates (those not descending from another candidate) the lowest `OpId`
//! wins; every losing branch is invalid, so concurrent `RemoveAdmin`s never both apply. The admin set and
//! threshold at X are the state after the latest chain link in X's causal past.
//!
//! **Stage A2, the account chain.** Account ops (`AddAccount`, `RemoveAccount`, `SetRole`,
//! `AuthorizeConsole`, `RevokeConsole`, `SetRecoveryKey`, and `Approved` wrapping one of
//! them) form a second linear chain from genesis, chosen exactly like stage A's. The state
//! at each link is the set of people (accounts with a role and passkeys), the authorized
//! consoles, and the recovery key. A candidate is accepted when:
//!
//! - it is `Approved` by a passkey of an account in that state, whose WebAuthn assertion
//!   signs the op's challenge (the [`OpId`](kith_types::OpId) it would have with the plain
//!   change as its body), or by the recovery key over the same challenge; and the
//!   approver's role allows the change: owners anything; admins adding non-owners,
//!   removing non-owners and revoking consoles; the recovery key adding people and
//!   authorizing consoles. The last owner can be neither removed nor demoted; or
//! - the state has no account yet, the op is a plain `AddAccount` with role owner or a plain
//!   `SetRecoveryKey` (while unset), and stage A's state at the op approves it as it would
//!   an authority op. After the first account, this device path is closed.
//!
//! Stage A2 depends only on stage A. Networks without account ops are unaffected by it.
//!
//! **Stage B, everything else**, in `(lamport, OpId)` order:
//!
//! - *Rule 1:* the author holds the needed role in X's causal past: admin for admin ops,
//!   member device for the sponsor of a `Join`. An `Acted` admin op, made for a person, may
//!   instead come from an authorized console, and its actor must hold an owner or admin
//!   role in X's causal past.
//! - *Rule 2, remove-wins:* X is invalid if a valid removal of its author's role
//!   (`RemoveAdmin` for admin ops, `RemoveDevice` of the sponsor for joins, and for an
//!   `Acted` op a chain link that revokes the console or takes the actor's role) is
//!   concurrent with X. Removals in X's past fall under rule 1, so a re-granted role can act
//!   again.
//!
//! **People's devices.** `AssignDevice` (an admin op) names a device's owner, last writer
//! wins after the device's admission; the account must exist at that point. A
//! `RemoveAccount` link removes, like a `RemoveDevice` there, every device last assigned to
//! the account in its causal past.
//!
//! **Membership.** A device is a member if some valid add of it (`AddDevice`, `Join`, or
//! being the genesis author) has every valid `RemoveDevice` of it strictly in its causal
//! past. A concurrent remove wins; a re-add after removal works.
//!
//! **Invites.** A `Join` also needs: the first valid `CreateInvite` for its key (by
//! `(lamport, OpId)`) in its causal past; a timestamp no later than the invite's expiry
//! (attested by the sponsor); no valid `RevokeInvite` of the key lacking the join in its
//! causal past (revoke-wins, forever); and a free use, counting otherwise-valid joins per
//! invite in `(lamport, OpId)` order up to `max_uses`.
//!
//! **Records** are last-writer-wins per key among valid `SetRecord` ops, by
//! `(lamport, OpId)`.
//!
//! **Structural rejection.** [`Ledger::insert`] rejects, and never stores, ops that are
//! malformed, oversized, badly signed, unsigned by their author, for another network, or
//! with the wrong lamport. Well-formed but unauthorized ops *are* stored, since a later op
//! can change what is valid; the view excludes them. Ops with unknown parents wait in a
//! bounded orphan buffer ([`MAX_ORPHANS`]).
//!
//! # Cost
//!
//! The view is recomputed from scratch after every change (once per
//! [`Ledger::insert_batch`]): `O(n log n)` to sort plus one `O(n + e)` walk per
//! authority- or account-chain step, per valid removal or revocation (two each), and per
//! `Join`. Each approval's signature is checked once per recompute. Memory stays `O(n)`
//! bits per walk.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod account;
mod error;
mod join;
mod ledger;
mod limits;
mod op;
mod validity;
mod view;

#[cfg(test)]
mod account_tests;
#[cfg(test)]
mod tests;

pub use account::{AccountChange, AdminChange, Approval, Assertion, Credential, Role, account_id};
pub use error::LedgerError;
pub use join::{AdmissionRequest, JoinProof};
pub use ledger::{BatchOutcome, InsertOutcome, Ledger};
pub use limits::{
    MAX_ADMINS, MAX_AUTHENTICATOR_DATA_LEN, MAX_CLIENT_DATA_LEN, MAX_CREDENTIAL_ID_LEN,
    MAX_NAME_LEN, MAX_ORPHANS, MAX_PARENTS, MAX_PASSKEY_SIG_LEN, MAX_RECORD_KEY_LEN,
    MAX_RECORD_VALUE_LEN, MAX_RP_ID_LEN, MAX_SIGNATURES, MAX_SIGNED_OP_LEN, MAX_TAG_LEN, MAX_TAGS,
};
pub use op::{NetworkParams, OpBody, SignedOp, UnsignedOp, client_id};
pub use view::{Account, Device, Invite, NetworkView, Via};
