//! Ledger reconciliation protocol between peers.
//!
//! Sans-IO: no I/O, no clock, no runtime. Two peers run one [`SyncSession`] each, per
//! network, and afterwards both hold the union of their op sets. The session borrows the
//! [`Ledger`](kith_ledger::Ledger) one call at a time.
//!
//! # Range-based set reconciliation
//!
//! As in negentropy (<https://logperiodic.com/rbsr.html>), cost grows with the size of the
//! difference times `log n`. Both sides look at the same half-open ranges of the 256-bit id
//! space. A side sends a [`fingerprint`] per range; a match is [`RangeItem::Skip`]. On a
//! mismatch, a side holding at most [`ID_LIST_THRESHOLD`] ids sends them
//! ([`RangeItem::IdList`]), otherwise it splits the range into [`BUCKETS`] buckets of equal
//! id count. A side receiving an id list pushes the ops the peer lacks and sends
//! [`Message::Want`] for the ones it lacks.
//!
//! Fingerprints hash the id count and the sum of ids mod 2^256, not an XOR: op authors
//! choose ids freely, and an XOR collision is just linear algebra over GF(2), while a sum
//! collision is subset-sum. The sum also forms a group, so a future incremental index can
//! answer range sums in `O(log n)`; today each round rescans the ids.
//!
//! # Messages and flights
//!
//! Messages are [`kith_wire`] envelopes of type
//! [`SYNC_MESSAGE`](kith_wire::registry::SYNC_MESSAGE), version [`SYNC_VERSION`], with every
//! list and the whole message bounded. Peers take turns: a *flight* is any number of
//! [`Message::Ops`] and [`Message::Want`], then one [`Message::Ranges`] or [`Message::Done`].
//! Wants and pushed ops are answered in the next flight, so crossing messages lose nothing.
//!
//! - The initiator opens with [`Message::Hello`]. Equal heads mean equal sets, so both are
//!   done after one round trip; otherwise the responder answers with a fingerprint of the
//!   whole space.
//! - A `Ranges` always covers the whole space; a tail over [`MAX_RANGE_ITEMS`] is folded
//!   into one fingerprint for the next round.
//! - A side receiving a bare `Done` with nothing to send is done, and confirms with a
//!   `Done` unless its last flight was already a bare `Done`.
//!
//! Ops travel as their exact signed bytes in lamport order, so orphan buffers stay small.
//! Ops the ledger refuses are counted in [`SyncStats::ops_rejected`] and skipped, so one bad
//! op cannot stop sync with an honest peer. A malformed or out-of-turn message, or more than
//! [`MAX_ROUNDS`] rounds, fails the session.

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
