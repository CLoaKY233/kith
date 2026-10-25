//! Ledger reconciliation protocol between peers.
//!
//! Plane: control. Sans-IO core: no I/O, no clock, no runtime. See `docs/architecture.md`.
//!
//! Every device keeps a full replica of its network's signed membership ledger. Two peers
//! that meet run one [`SyncSession`] each, per network, and afterwards both hold the union
//! of their op sets. The session never owns the [`Ledger`](kith_ledger::Ledger): it borrows
//! it for one call at a time, so the driver stays free to read the view between messages.
//!
//! # Why range-based set reconciliation
//!
//! Two replicas usually differ by a handful of recent ops, but a freshly joined device
//! differs by the whole history. Range-based set reconciliation (as in negentropy, see
//! <https://logperiodic.com/rbsr.html>) handles both: the cost grows with the size of the
//! difference times the logarithm of the set, not with the set itself.
//!
//! Op ids are BLAKE3 hashes, so they are uniformly spread over the 256-bit space. Each
//! side looks at the same half-open ranges of that space, sorted by id:
//!
//! 1. A side sends a [`fingerprint`] of a range: a BLAKE3 hash under
//!    [`FINGERPRINT_CONTEXT`] of the range's id count and the sum of its ids mod 2^256.
//! 2. The other side compares it with its own fingerprint of that range. A match means the
//!    range is in sync ([`RangeItem::Skip`]).
//! 3. On a mismatch, a side holding at most [`ID_LIST_THRESHOLD`] ids in the range sends
//!    them explicitly ([`RangeItem::IdList`]). Otherwise it splits the range into
//!    [`BUCKETS`] buckets of equal id count and fingerprints each one.
//! 4. A side receiving an id list knows the whole difference for that range: it pushes the
//!    ops the peer lacks and sends [`Message::Want`] for the ones it lacks.
//!
//! Each split divides the splitting side's id count by [`BUCKETS`], so a range resolves in
//! `O(log n)` rounds.
//!
//! ## Why a sum and not an XOR
//!
//! Anyone who can author ops chooses their ids freely by varying the content. With an XOR,
//! finding a subset of ids that cancels out (so two different sets fingerprint the same) is
//! linear algebra over GF(2): about 257 candidate ops and Gaussian elimination. With a sum
//! mod 2^256 it is a subset-sum problem, which is far harder. The id count is hashed in too,
//! so sets of different sizes never collide by construction. Both choices form an abelian
//! group, which is what the planned incremental index needs: a tree of per-node partial sums
//! can be updated in `O(log n)` per insert and answer any range's sum without rescanning.
//! Today each round rescans the ids, which is linear but fine at ledger sizes.
//!
//! # Messages and flights
//!
//! Messages travel in a [`kith_wire`] envelope of type
//! [`SYNC_MESSAGE`](kith_wire::registry::SYNC_MESSAGE), version [`SYNC_VERSION`], with a
//! postcard body. Every list is bounded ([`MAX_HEADS`], [`MAX_RANGE_ITEMS`],
//! [`MAX_IDS_PER_MESSAGE`], [`MAX_OPS_PER_MESSAGE`]) and so is the whole message
//! ([`MAX_SYNC_MESSAGE_LEN`]).
//!
//! The peers take turns. A *flight* is everything one side sends in its turn: any number of
//! [`Message::Ops`] and [`Message::Want`] messages, then exactly one terminator,
//! [`Message::Ranges`] or [`Message::Done`]. Wants and pushed ops are answered in the next
//! flight, so nothing is ever lost to crossing messages.
//!
//! - The initiator opens with [`Message::Hello`]. If both sides' DAG heads match, the sets
//!   are identical (every stored op is an ancestor of a head) and both are done after one
//!   round trip. Otherwise the responder answers with its `Hello` and a fingerprint of the
//!   whole id space.
//! - A [`Message::Ranges`] always covers the whole id space. When a reply would exceed
//!   [`MAX_RANGE_ITEMS`], its tail is folded into one fingerprint, to be split next round.
//! - A side sends `Done` when it has no unresolved ranges. Once a side receives a `Done`
//!   flight with nothing else in it and has nothing to send, it is done. It confirms with a
//!   `Done` of its own unless its last flight was already an empty `Done`.
//!
//! Ops travel as their exact signed bytes and are sent in causal order (by lamport clock)
//! so the receiver's orphan buffer stays small. The receiver stores them with
//! [`Ledger::insert_batch`](kith_ledger::Ledger::insert_batch). Ops that fail insertion are
//! counted in [`SyncStats::ops_rejected`] and skipped: one bad op must not stop sync with
//! an otherwise honest peer. When an op is orphaned, its missing ancestors are requested
//! explicitly too.
//!
//! A malformed or out-of-turn message fails the session. So does a peer that keeps the
//! reconciliation going for more than [`MAX_ROUNDS`] rounds.

#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::arithmetic_side_effects)]

extern crate alloc;

mod error;
mod fingerprint;
mod message;
mod session;

#[cfg(test)]
mod tests;

pub use error::SyncError;
pub use fingerprint::{FINGERPRINT_CONTEXT, Fingerprint, fingerprint};
pub use message::{Bound, Message, RangeItem};
pub use session::{SyncSession, SyncStats};

/// The schema version of [`Message`] bodies.
pub const SYNC_VERSION: u64 = 1;

/// How many buckets a mismatched range is split into.
pub const BUCKETS: usize = 16;

/// A mismatched range holding at most this many ids is sent as an explicit id list.
pub const ID_LIST_THRESHOLD: usize = 32;

/// The most heads a [`Message::Hello`] carries. A ledger with more sends none, which
/// disables the in-sync fast path but nothing else.
pub const MAX_HEADS: usize = 256;

/// The most items in one [`Message::Ranges`].
pub const MAX_RANGE_ITEMS: usize = 256;

/// The most op ids in one message: in a [`Message::Want`], or across all id lists of one
/// [`Message::Ranges`]. `MAX_RANGE_ITEMS * ID_LIST_THRESHOLD` fits exactly.
pub const MAX_IDS_PER_MESSAGE: usize = 8192;

/// The most ops in one [`Message::Ops`].
pub const MAX_OPS_PER_MESSAGE: usize = 256;

/// The largest encoded sync message, envelope included: 512 KiB, half of
/// [`kith_wire::envelope::MAX_MESSAGE_LEN`]. Room for several maximum-size ops, and for a
/// full [`Message::Ranges`] (about 280 KiB).
pub const MAX_SYNC_MESSAGE_LEN: usize = 512 * 1024;

/// The most rounds (received `Hello`, `Ranges` and `Done` messages) one session accepts.
/// Honest peers need about `2 * log16(n) + 6`; the rest is margin.
pub const MAX_ROUNDS: u32 = 64;

const _: () = assert!(MAX_RANGE_ITEMS * ID_LIST_THRESHOLD <= MAX_IDS_PER_MESSAGE);
const _: () = assert!(MAX_SYNC_MESSAGE_LEN <= kith_wire::envelope::MAX_MESSAGE_LEN);
const _: () = assert!(MAX_RANGE_ITEMS >= 2 && BUCKETS >= 2 && ID_LIST_THRESHOLD >= 1);
