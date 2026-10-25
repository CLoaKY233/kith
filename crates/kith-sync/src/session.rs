//! The per-peer sync state machine.

use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec::Vec;

use kith_ledger::{InsertOutcome, Ledger, SignedOp};
use kith_types::{NetworkId, OpId};

use crate::{
    BUCKETS, Bound, ID_LIST_THRESHOLD, MAX_HEADS, MAX_IDS_PER_MESSAGE, MAX_OPS_PER_MESSAGE,
    MAX_RANGE_ITEMS, MAX_ROUNDS, MAX_SYNC_MESSAGE_LEN, Message, RangeItem, SyncError, fingerprint,
};

/// Room left in an [`Message::Ops`] for the envelope and the list length.
const OPS_MESSAGE_OVERHEAD: usize = 32;

/// Postcard's length prefix for one op is at most this long (ops are under 2^21 bytes).
const OP_LENGTH_PREFIX: usize = 3;

/// Counters for one session, for logs and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// `Hello`, `Ranges` and `Done` messages received; capped by [`MAX_ROUNDS`].
    pub rounds: u32,
    /// Messages received and accepted.
    pub messages_received: u64,
    /// Messages handed out by [`SyncSession::poll_transmit`].
    pub messages_sent: u64,
    /// Ops sent to the peer.
    pub ops_sent: u64,
    /// Received ops the ledger stored.
    pub ops_inserted: u64,
    /// Received ops the ledger buffered as orphans.
    pub ops_orphaned: u64,
    /// Received ops the ledger already had.
    pub ops_duplicate: u64,
    /// Received ops that did not parse or that the ledger refused.
    pub ops_rejected: u64,
    /// Op ids asked of the peer.
    pub ids_requested: u64,
}

/// Where the session is in the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Initiator only: the `Hello` is not queued yet.
    Start,
    /// Waiting for the peer's `Hello`.
    AwaitHello,
    /// Reconciling ranges and moving ops.
    Reconciling,
    /// Both sides hold the same ops; nothing more is sent or accepted.
    Finished,
    /// The peer broke the protocol.
    Failed,
}

/// One sync session with one peer for one network.
///
/// Feed every received message to [`handle_message`](Self::handle_message) and send
/// whatever [`poll_transmit`](Self::poll_transmit) returns, in order, until
/// [`is_done`](Self::is_done). The session borrows the ledger only for each call.
#[derive(Debug)]
pub struct SyncSession {
    initiator: bool,
    network: NetworkId,
    phase: Phase,
    outbox: VecDeque<Vec<u8>>,
    /// Ids the peer asked for in its current flight.
    peer_wants: BTreeSet<OpId>,
    /// Ids the peer lacks, found in its id lists.
    to_push: BTreeSet<OpId>,
    /// Ids to ask for in the next flight.
    to_request: BTreeSet<OpId>,
    /// Every id ever asked for, so nothing is asked twice.
    requested: BTreeSet<OpId>,
    /// Every op ever sent, so nothing is sent twice.
    sent: BTreeSet<OpId>,
    /// Whether the peer's current flight carried ops or wants.
    peer_flight_has_content: bool,
    /// Whether our last flight was a bare `Done`.
    last_flight_quiet: bool,
    stats: SyncStats,
}

impl SyncSession {
    /// A session for `network`. Exactly one of the two peers is the `initiator`; it sends
    /// the first message.
    pub const fn new(initiator: bool, network: NetworkId) -> Self {
        Self {
            initiator,
            network,
            phase: if initiator {
                Phase::Start
            } else {
                Phase::AwaitHello
            },
            outbox: VecDeque::new(),
            peer_wants: BTreeSet::new(),
            to_push: BTreeSet::new(),
            to_request: BTreeSet::new(),
            requested: BTreeSet::new(),
            sent: BTreeSet::new(),
            peer_flight_has_content: false,
            last_flight_quiet: false,
            stats: SyncStats {
                rounds: 0,
                messages_received: 0,
                messages_sent: 0,
                ops_sent: 0,
                ops_inserted: 0,
                ops_orphaned: 0,
                ops_duplicate: 0,
                ops_rejected: 0,
                ids_requested: 0,
            },
        }
    }

    /// Whether sync finished and everything queued was sent: both ledgers now hold the
    /// same ops (except ops neither side could supply).
    pub fn is_done(&self) -> bool {
        self.phase == Phase::Finished && self.outbox.is_empty()
    }

    /// The session's counters.
    pub const fn stats(&self) -> &SyncStats {
        &self.stats
    }

    /// The next message to send, if any.
    pub fn poll_transmit(&mut self, ledger: &Ledger) -> Option<Vec<u8>> {
        if self.phase == Phase::Start && ledger.network_id() == self.network {
            if let Err(_error) = self.queue(&self.hello(ledger)) {
                self.fail();
                return None;
            }
            self.phase = Phase::AwaitHello;
        }
        let message = self.outbox.pop_front()?;
        self.stats.messages_sent = self.stats.messages_sent.saturating_add(1);
        Some(message)
    }

    /// Handles one message from the peer, storing any ops it carries in `ledger`.
    ///
    /// Ops the ledger refuses are counted and skipped. Any other error fails the session
    /// (except [`SyncError::LedgerMismatch`], which leaves it untouched), and the ledger
    /// keeps whatever valid ops arrived before.
    pub fn handle_message(&mut self, ledger: &mut Ledger, bytes: &[u8]) -> Result<(), SyncError> {
        if ledger.network_id() != self.network {
            return Err(SyncError::LedgerMismatch);
        }
        if matches!(self.phase, Phase::Finished | Phase::Failed) {
            return Err(SyncError::Closed);
        }
        let result = Message::decode(bytes).and_then(|message| self.dispatch(ledger, message));
        match result {
            Ok(()) => {
                self.stats.messages_received = self.stats.messages_received.saturating_add(1);
            }
            Err(_) => self.fail(),
        }
        result
    }

    fn dispatch(&mut self, ledger: &mut Ledger, message: Message) -> Result<(), SyncError> {
        match (self.phase, message) {
            (Phase::AwaitHello, Message::Hello { network, heads }) => {
                self.next_round()?;
                self.handle_hello(ledger, network, &heads)
            }
            (Phase::Reconciling, Message::Ops(ops)) => {
                self.handle_ops(ledger, &ops);
                Ok(())
            }
            (Phase::Reconciling, Message::Want(ids)) => {
                self.peer_flight_has_content = true;
                self.peer_wants.extend(ids);
                Ok(())
            }
            (Phase::Reconciling, Message::Ranges(items)) => {
                self.next_round()?;
                let reply = self.reconcile(ledger, &items);
                self.end_flight(ledger, reply)
            }
            (Phase::Reconciling, Message::Done) => {
                self.next_round()?;
                self.handle_done(ledger)
            }
            _ => Err(SyncError::UnexpectedMessage),
        }
    }

    fn next_round(&mut self) -> Result<(), SyncError> {
        self.stats.rounds = self.stats.rounds.saturating_add(1);
        if self.stats.rounds > MAX_ROUNDS {
            Err(SyncError::TooManyRounds)
        } else {
            Ok(())
        }
    }

    fn handle_hello(
        &mut self,
        ledger: &Ledger,
        network: NetworkId,
        heads: &[OpId],
    ) -> Result<(), SyncError> {
        if network != self.network {
            return Err(SyncError::WrongNetwork {
                expected: self.network,
                found: network,
            });
        }
        // Equal heads mean equal sets: every stored op is an ancestor of a head.
        let in_sync = !heads.is_empty() && heads == ledger.heads();
        if !self.initiator {
            self.queue(&self.hello(ledger))?;
        }
        if in_sync {
            self.phase = Phase::Finished;
            return Ok(());
        }
        self.phase = Phase::Reconciling;
        if !self.initiator {
            let ids: Vec<OpId> = ledger.op_ids().copied().collect();
            self.queue(&Message::Ranges(Vec::from([RangeItem::Fingerprint {
                upper_bound: Bound::End,
                fingerprint: fingerprint(&ids),
            }])))?;
        }
        Ok(())
    }

    fn hello(&self, ledger: &Ledger) -> Message {
        let heads = ledger.heads();
        Message::Hello {
            network: self.network,
            heads: if heads.len() <= MAX_HEADS {
                heads.to_vec()
            } else {
                Vec::new()
            },
        }
    }

    fn handle_ops(&mut self, ledger: &mut Ledger, raw: &[Vec<u8>]) {
        self.peer_flight_has_content = true;
        let mut ops = Vec::with_capacity(raw.len());
        for bytes in raw {
            match SignedOp::from_bytes(bytes) {
                Ok(op) => ops.push(op),
                Err(_) => self.stats.ops_rejected = self.stats.ops_rejected.saturating_add(1),
            }
        }
        for result in ledger.insert_batch(ops).results {
            let counter = match result {
                Ok(InsertOutcome::Inserted) => &mut self.stats.ops_inserted,
                Ok(InsertOutcome::Duplicate | InsertOutcome::SignaturesAdded) => {
                    &mut self.stats.ops_duplicate
                }
                Ok(InsertOutcome::Orphaned { missing }) => {
                    for id in missing {
                        self.request(ledger, id);
                    }
                    &mut self.stats.ops_orphaned
                }
                Err(_) => &mut self.stats.ops_rejected,
            };
            *counter = counter.saturating_add(1);
        }
    }

    fn handle_done(&mut self, ledger: &Ledger) -> Result<(), SyncError> {
        let peer_quiet = !self.peer_flight_has_content;
        if !self.flight_contents(ledger).is_empty() || !peer_quiet {
            // Either we still have something to say, or the peer does and needs a turn.
            return self.end_flight(ledger, None);
        }
        // The peer said a bare `Done` and we have nothing: sync is over. The peer is
        // waiting for our confirmation unless our last flight was a bare `Done` too.
        if !self.last_flight_quiet {
            self.queue(&Message::Done)?;
        }
        self.phase = Phase::Finished;
        Ok(())
    }

    /// Queues our flight: ops answering the peer, our wants, then `Ranges` or `Done`.
    fn end_flight(
        &mut self,
        ledger: &Ledger,
        ranges: Option<Vec<RangeItem>>,
    ) -> Result<(), SyncError> {
        let contents = self.flight_contents(ledger);
        self.sent.extend(
            self.peer_wants
                .iter()
                .chain(&self.to_push)
                .filter(|id| ledger.contains(id)),
        );
        self.last_flight_quiet = contents.is_empty() && ranges.is_none();
        for message in &contents {
            if let Message::Ops(ops) = message {
                let count = u64::try_from(ops.len()).unwrap_or(u64::MAX);
                self.stats.ops_sent = self.stats.ops_sent.saturating_add(count);
            }
            self.queue(message)?;
        }
        self.queue(&ranges.map_or(Message::Done, Message::Ranges))?;
        self.peer_wants.clear();
        self.to_push.clear();
        self.to_request.clear();
        self.peer_flight_has_content = false;
        Ok(())
    }

    /// The ops and wants of our next flight, without consuming them.
    fn flight_contents(&self, ledger: &Ledger) -> Vec<Message> {
        let mut ops: Vec<&SignedOp> = self
            .peer_wants
            .iter()
            .chain(&self.to_push)
            .filter(|id| !self.sent.contains(*id))
            .filter_map(|id| ledger.get(id))
            .collect();
        // Causal order: parents always have smaller lamport clocks.
        ops.sort_by_key(|op| (op.op().lamport, op.id()));
        ops.dedup_by_key(|op| op.id());

        let mut messages = Vec::new();
        let mut batch: Vec<Vec<u8>> = Vec::new();
        let mut batch_bytes = 0_usize;
        let budget = MAX_SYNC_MESSAGE_LEN.saturating_sub(OPS_MESSAGE_OVERHEAD);
        for op in ops {
            let bytes = op.to_bytes();
            let cost = bytes.len().saturating_add(OP_LENGTH_PREFIX);
            if !batch.is_empty()
                && (batch.len() >= MAX_OPS_PER_MESSAGE || batch_bytes.saturating_add(cost) > budget)
            {
                messages.push(Message::Ops(core::mem::take(&mut batch)));
                batch_bytes = 0;
            }
            batch_bytes = batch_bytes.saturating_add(cost);
            batch.push(bytes);
        }
        if !batch.is_empty() {
            messages.push(Message::Ops(batch));
        }
        let wants: Vec<OpId> = self.to_request.iter().copied().collect();
        messages.extend(
            wants
                .chunks(MAX_IDS_PER_MESSAGE)
                .map(|chunk| Message::Want(chunk.to_vec())),
        );
        messages
    }

    /// Answers the peer's ranges. `None` means every range is in sync.
    fn reconcile(&mut self, ledger: &Ledger, items: &[RangeItem]) -> Option<Vec<RangeItem>> {
        let ids: Vec<OpId> = ledger.op_ids().copied().collect();
        let mut reply = Vec::new();
        let mut start = 0_usize;
        for item in items {
            let upper_bound = item.upper_bound();
            let end = ids.partition_point(|id| upper_bound.is_above(id));
            let mine = ids.get(start..end).unwrap_or_default();
            start = end;
            match item {
                RangeItem::Skip { .. } => push_skip(&mut reply, upper_bound),
                RangeItem::Fingerprint {
                    fingerprint: theirs,
                    ..
                } => {
                    if fingerprint(mine) == *theirs {
                        push_skip(&mut reply, upper_bound);
                    } else if mine.len() <= ID_LIST_THRESHOLD {
                        reply.push(RangeItem::IdList {
                            upper_bound,
                            ids: mine.to_vec(),
                        });
                    } else {
                        split(mine, upper_bound, &mut reply);
                    }
                }
                RangeItem::IdList { ids: theirs, .. } => {
                    self.compare(ledger, mine, theirs);
                    push_skip(&mut reply, upper_bound);
                }
            }
        }
        if reply.len() > MAX_RANGE_ITEMS {
            // Fold the tail into one range, to be split again next round.
            reply.truncate(MAX_RANGE_ITEMS.saturating_sub(1));
            let tail_start = match reply.last().map(RangeItem::upper_bound) {
                Some(Bound::Id(lower)) => ids.partition_point(|id| *id < lower),
                Some(Bound::End) | None => 0,
            };
            reply.push(RangeItem::Fingerprint {
                upper_bound: Bound::End,
                fingerprint: fingerprint(ids.get(tail_start..).unwrap_or_default()),
            });
        }
        let all_skipped = reply
            .iter()
            .all(|item| matches!(item, RangeItem::Skip { .. }));
        (!all_skipped).then_some(reply)
    }

    /// Records the difference between our ids and the peer's in one range. Both are sorted.
    fn compare(&mut self, ledger: &Ledger, mine: &[OpId], theirs: &[OpId]) {
        let mut mine = mine.iter().peekable();
        let mut theirs = theirs.iter().peekable();
        loop {
            match (mine.peek(), theirs.peek()) {
                (Some(&&own), Some(&&other)) if own == other => {
                    mine.next();
                    theirs.next();
                }
                (Some(&&own), Some(&&other)) if own < other => {
                    self.to_push.insert(own);
                    mine.next();
                }
                (Some(&&own), None) => {
                    self.to_push.insert(own);
                    mine.next();
                }
                (_, Some(&&other)) => {
                    self.request(ledger, other);
                    theirs.next();
                }
                (None, None) => break,
            }
        }
    }

    /// Asks the peer for `id` in the next flight, unless we have it or asked before.
    fn request(&mut self, ledger: &Ledger, id: OpId) {
        if !ledger.contains(&id) && self.requested.insert(id) {
            self.to_request.insert(id);
            self.stats.ids_requested = self.stats.ids_requested.saturating_add(1);
        }
    }

    fn queue(&mut self, message: &Message) -> Result<(), SyncError> {
        self.outbox.push_back(message.encode()?);
        Ok(())
    }

    fn fail(&mut self) {
        self.phase = Phase::Failed;
        self.outbox.clear();
    }
}

/// Appends an in-sync range, merging it with a preceding one.
fn push_skip(reply: &mut Vec<RangeItem>, upper_bound: Bound) {
    if let Some(RangeItem::Skip { upper_bound: last }) = reply.last_mut() {
        *last = upper_bound;
    } else {
        reply.push(RangeItem::Skip { upper_bound });
    }
}

/// Splits `ids` (more than [`ID_LIST_THRESHOLD`]) into [`BUCKETS`] ranges whose counts
/// differ by at most one.
fn split(ids: &[OpId], upper_bound: Bound, reply: &mut Vec<RangeItem>) {
    let boundary = |bucket: usize| {
        ids.len()
            .checked_mul(bucket)
            .and_then(|scaled| scaled.checked_div(BUCKETS))
            .unwrap_or(ids.len())
    };
    for bucket in 0..BUCKETS {
        let start = boundary(bucket);
        let end = boundary(bucket.saturating_add(1));
        let bound = ids.get(end).map_or(upper_bound, |next| Bound::Id(*next));
        reply.push(RangeItem::Fingerprint {
            upper_bound: bound,
            fingerprint: fingerprint(ids.get(start..end).unwrap_or_default()),
        });
    }
}
