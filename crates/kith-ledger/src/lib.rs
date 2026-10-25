//! The signed, causally ordered membership ledger and the network view derived from it.
//!
//! Plane: trust. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! # Why a ledger
//!
//! kith has no coordination server. Every device keeps a full replica of a signed,
//! append-only DAG of operations ("ops"), and derives the network state — admins, member
//! devices, invites, records — from it. Replicas exchange ops in any order, and any two
//! replicas holding the same ops derive the same [`NetworkView`]. Nothing depends on
//! insertion order or on a clock.
//!
//! # Ops
//!
//! An op names its network, its parents (the DAG tips its author had seen, 1 to
//! [`MAX_PARENTS`], sorted), its author, a lamport clock that is exactly one more than the
//! largest parent's, the author's timestamp, and a body ([`OpBody`]). The genesis op has
//! no network and no parents, and lamport 0; its [`OpId`](kith_types::OpId) *is* the
//! [`NetworkId`](kith_types::NetworkId). See [`op`] for the exact encoding, and [`limits`]
//! for the size limits enforced when decoding.
//!
//! A [`SignedOp`] keeps the exact bytes it was decoded from, and verifies signatures over
//! those bytes; it never re-encodes. Signatures sit outside the hashed bytes, so admins can
//! co-sign one op ([`SignedOp::add_signature`]) and replicas merge co-signatures of the same
//! op when they meet.
//!
//! # Validity
//!
//! "X's causal past" means X's strict ancestors. Validity is decided in two stages, so
//! that no rule ever depends on itself.
//!
//! **Stage A, the authority chain.** Genesis, `AddAdmin`, `RemoveAdmin` and `SetThreshold`
//! form a linear chain starting at genesis. The next link after chain head H is chosen
//! among the authority ops that have H in their causal past, are authored by an admin of
//! the state at H, are signed by at least the threshold of distinct admins of that state,
//! and make an allowed change (no duplicate admin, no removal below the threshold, a
//! threshold between 1 and the admin count). Only H's *direct* children compete: an op
//! built on top of another candidate belongs to that candidate's branch. If several
//! compete, the one with the lowest `OpId` wins, and every losing branch is invalid. The
//! admin set and threshold at any op X are the state after the latest chain link in X's
//! causal past.
//!
//! **Stage B, everything else**, in the total order `(lamport, OpId)`, which is
//! consistent with causality:
//!
//! - *Rule 1:* the author holds the needed role in X's causal past: admin (per stage A) for
//!   admin ops, member device for the sponsor of a `Join`.
//! - *Rule 2, remove-wins:* X is invalid if a valid removal of its author's role
//!   (`RemoveAdmin` for admin ops, `RemoveDevice` of the sponsor for joins) is concurrent
//!   with X. Removals strictly in X's past are handled by rule 1, which lets a role that
//!   was removed and then granted again act after the re-grant.
//! - Removals are subject to rule 1 only, so concurrent removals of each other's devices
//!   all apply.
//!
//! **Membership.** A device is a member if there is a valid add of it (`AddDevice`, `Join`,
//! or being the genesis author) such that every valid `RemoveDevice` of it is strictly in
//! that add's causal past. A remove concurrent with an add wins; a re-add after a removal
//! works.
//!
//! **Invites.** A `Join` additionally needs: the first valid `CreateInvite` for its key (in
//! `(lamport, OpId)` order) in its causal past; a timestamp no later than that invite's
//! expiry (the sponsor attests to the time); no valid `RevokeInvite` of the key that lacks
//! the join in its causal past (revoke-wins, so a revoked key is dead forever); and a free
//! use: otherwise-valid joins are counted per invite in `(lamport, OpId)` order, and those
//! beyond `max_uses` are invalid.
//!
//! **Records** are last-writer-wins per key among valid `SetRecord` ops, by
//! `(lamport, OpId)`.
//!
//! **Structural rejection.** [`Ledger::insert`] rejects with a [`LedgerError`], and never
//! stores, ops that are malformed, oversized, badly signed, unsigned by their author, for
//! another network, or with the wrong lamport clock. Well-formed but unauthorized ops *are*
//! stored, because a later op can change what is valid (a fork resolution, say); the view
//! excludes them. Ops with unknown parents wait in a bounded orphan buffer
//! ([`MAX_ORPHANS`]) and are stored automatically when their parents arrive.
//!
//! # Cost
//!
//! The view is recomputed from scratch after every change, or once per
//! [`Ledger::insert_batch`]. With `n` ops and `e` parent
//! edges, that is `O(n log n)` to sort, plus one breadth-first walk (`O(n + e)` time, an
//! `n`-bit set of memory) per authority-chain step, per valid device removal or invite
//! revocation (two walks each), and per `Join`. Memory is `O(n)` bits per removal or
//! revocation, never `O(n²)`. That is fine for the thousands of ops a network holds.
//!
//! The obvious incremental approach for later: since stage B runs in topological order,
//! a new op whose lamport is above every stored op only needs its own verdict and the
//! verdicts of nothing else, unless it is a removal, revocation or authority op; those
//! (and late arrivals) can trigger a re-evaluation from the earliest affected position
//! instead of from genesis.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod error;
mod join;
mod ledger;
pub mod limits;
pub mod op;
mod validity;
mod view;

#[cfg(test)]
mod tests;

pub use error::LedgerError;
pub use join::{DEVICE_CONSENT_CONTEXT, INVITE_REDEEM_CONTEXT, JoinProof};
pub use ledger::{BatchOutcome, InsertOutcome, Ledger};
pub use limits::{
    MAX_ADMINS, MAX_NAME_LEN, MAX_OP_LEN, MAX_ORPHANS, MAX_PARENTS, MAX_RECORD_KEY_LEN,
    MAX_RECORD_VALUE_LEN, MAX_SIGNATURES, MAX_SIGNED_OP_LEN, MAX_TAG_LEN, MAX_TAGS,
};
pub use op::{NetworkParams, OpBody, SignedOp, UnsignedOp};
pub use view::{Device, Invite, NetworkView};
