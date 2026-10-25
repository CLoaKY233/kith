//! The replica: stored ops, the orphan buffer, and the derived view.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use kith_crypto::SigningKey;
use kith_types::{NetworkId, NodeId, OpId, Timestamp};

use crate::limits::{MAX_ORPHANS, MAX_PARENTS};
use crate::op::Kind;
use crate::validity::evaluate;
use crate::view::NetworkView;
use crate::{LedgerError, NetworkParams, OpBody, SignedOp, UnsignedOp};

/// What [`Ledger::insert`] did with an op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    /// The op was stored (and so were any orphans that were waiting for it).
    Inserted,
    /// The op was already stored or buffered, with no new signatures.
    Duplicate,
    /// The op was already stored or buffered; its new co-signatures were merged in.
    SignaturesAdded,
    /// Some ancestors are unknown; the op is buffered until they arrive.
    Orphaned {
        /// Ancestors that are neither stored nor buffered, sorted. Fetch these.
        missing: Vec<OpId>,
    },
}

impl InsertOutcome {
    /// Whether the stored ops changed, so the view must be recomputed.
    const fn changes_view(&self) -> bool {
        matches!(self, Self::Inserted | Self::SignaturesAdded)
    }
}

/// What [`Ledger::insert_batch`] did with each op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchOutcome {
    /// What [`Ledger::insert`] would have returned for each op, in input order.
    pub results: Vec<Result<InsertOutcome, LedgerError>>,
}

impl BatchOutcome {
    /// The first rejection, if any.
    pub fn first_error(&self) -> Option<&LedgerError> {
        self.results.iter().find_map(|result| result.as_ref().err())
    }

    /// How many ops were stored (not counting orphans adopted along the way).
    pub fn inserted(&self) -> usize {
        self.results
            .iter()
            .filter(|result| matches!(result, Ok(InsertOutcome::Inserted)))
            .count()
    }
}

/// One device's replica of a network's membership ledger and its derived [`NetworkView`].
#[derive(Debug, Clone)]
pub struct Ledger {
    genesis: OpId,
    founder: NodeId,
    ops: BTreeMap<OpId, SignedOp>,
    heads: BTreeSet<OpId>,
    orphans: BTreeMap<OpId, SignedOp>,
    genesis_view: NetworkView,
    view: NetworkView,
}

impl Ledger {
    /// Opens a ledger rooted at `genesis`.
    pub fn new(genesis: SignedOp) -> Result<Self, LedgerError> {
        let OpBody::Genesis {
            name,
            admins,
            threshold,
            params,
            founder_name: _,
        } = &genesis.op().body
        else {
            return Err(LedgerError::NotGenesis);
        };
        let id = genesis.id();
        let genesis_view = NetworkView {
            network: network_of(id),
            name: name.clone(),
            params: *params,
            admins: admins.iter().copied().collect(),
            threshold: *threshold,
            devices: BTreeMap::new(),
            invites: BTreeMap::new(),
            records: BTreeMap::new(),
            heads: Vec::new(),
        };
        let founder = genesis.op().author;
        let mut ledger = Self {
            genesis: id,
            founder,
            ops: BTreeMap::from([(id, genesis)]),
            heads: BTreeSet::from([id]),
            orphans: BTreeMap::new(),
            view: genesis_view.clone(),
            genesis_view,
        };
        ledger.recompute();
        Ok(ledger)
    }

    /// Founds a new network. The founder is always an admin; `extra_admins` are deduplicated.
    #[expect(
        clippy::too_many_arguments,
        reason = "each argument is one genesis field"
    )]
    pub fn create(
        founder: &SigningKey,
        name: &str,
        founder_name: &str,
        params: NetworkParams,
        extra_admins: &[NodeId],
        threshold: u8,
        timestamp: Timestamp,
    ) -> Result<Self, LedgerError> {
        let mut admins: BTreeSet<NodeId> = extra_admins.iter().copied().collect();
        admins.insert(founder.node_id());
        let genesis = UnsignedOp {
            network: None,
            parents: Vec::new(),
            author: founder.node_id(),
            lamport: 0,
            timestamp,
            body: OpBody::Genesis {
                name: String::from(name),
                admins: admins.into_iter().collect(),
                threshold,
                params,
                founder_name: String::from(founder_name),
            },
        };
        Self::new(genesis.sign(founder)?)
    }

    /// Adds an op received from anywhere. Unauthorized but well-formed ops are stored.
    pub fn insert(&mut self, op: SignedOp) -> Result<InsertOutcome, LedgerError> {
        let outcome = self.insert_without_recompute(op)?;
        if outcome.changes_view() {
            self.recompute();
        }
        Ok(outcome)
    }

    /// Adds many ops as [`insert`](Self::insert) would, recomputing the view once.
    ///
    /// A rejected op does not stop the batch, so one bad op from a peer cannot discard the
    /// good ones around it.
    pub fn insert_batch(&mut self, ops: impl IntoIterator<Item = SignedOp>) -> BatchOutcome {
        let results: Vec<Result<InsertOutcome, LedgerError>> = ops
            .into_iter()
            .map(|op| self.insert_without_recompute(op))
            .collect();
        if results
            .iter()
            .any(|result| result.as_ref().is_ok_and(InsertOutcome::changes_view))
        {
            self.recompute();
        }
        BatchOutcome { results }
    }

    fn insert_without_recompute(&mut self, op: SignedOp) -> Result<InsertOutcome, LedgerError> {
        if op.op().body.kind() == Kind::Genesis {
            return if op.id() == self.genesis {
                Ok(self.merge_copy(&op))
            } else {
                Err(LedgerError::UnexpectedGenesis)
            };
        }
        if op.op().network != Some(self.network_id()) {
            return Err(LedgerError::NetworkMismatch);
        }
        if self.ops.contains_key(&op.id()) {
            return Ok(self.merge_copy(&op));
        }
        if let Some(buffered) = self.orphans.get_mut(&op.id()) {
            buffered.merge_signatures(&op);
            return Ok(InsertOutcome::Orphaned {
                missing: self.missing_ancestors(&op),
            });
        }
        if op
            .op()
            .parents
            .iter()
            .any(|parent| !self.ops.contains_key(parent))
        {
            if self.orphans.len() >= MAX_ORPHANS {
                return Err(LedgerError::OrphanBufferFull);
            }
            let missing = self.missing_ancestors(&op);
            self.orphans.insert(op.id(), op);
            return Ok(InsertOutcome::Orphaned { missing });
        }
        self.attach(op)?;
        self.adopt_orphans();
        Ok(InsertOutcome::Inserted)
    }

    /// Drafts an op by `author` on up to [`MAX_PARENTS`] current heads (the lowest ids).
    pub fn draft(&self, author: NodeId, body: OpBody, timestamp: Timestamp) -> UnsignedOp {
        let parents: Vec<OpId> = self.heads.iter().copied().take(MAX_PARENTS).collect();
        let lamport = parents
            .iter()
            .filter_map(|parent| self.ops.get(parent))
            .map(|op| op.op().lamport)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        UnsignedOp {
            network: Some(self.network_id()),
            parents,
            author,
            lamport,
            timestamp,
            body,
        }
    }

    /// The network's identity: the genesis op's id.
    pub const fn network_id(&self) -> NetworkId {
        self.genesis_view.network
    }

    /// The genesis op's id.
    pub const fn genesis_id(&self) -> OpId {
        self.genesis
    }

    /// The DAG tips, sorted.
    pub fn heads(&self) -> &[OpId] {
        self.view.heads()
    }

    /// A stored op.
    pub fn get(&self, id: &OpId) -> Option<&SignedOp> {
        self.ops.get(id)
    }

    /// Whether `id` is stored.
    pub fn contains(&self, id: &OpId) -> bool {
        self.ops.contains_key(id)
    }

    /// Every stored op id, ascending. Buffered orphans are not included.
    pub fn op_ids(&self) -> impl Iterator<Item = &OpId> + '_ {
        self.ops.keys()
    }

    /// How many ops are stored.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Always false: a ledger holds at least its genesis.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// How many ops are waiting for their parents.
    pub fn orphan_count(&self) -> usize {
        self.orphans.len()
    }

    /// The network state derived from the valid ops.
    pub const fn view(&self) -> &NetworkView {
        &self.view
    }

    fn merge_copy(&mut self, op: &SignedOp) -> InsertOutcome {
        let changed = self
            .ops
            .get_mut(&op.id())
            .is_some_and(|stored| stored.merge_signatures(op));
        if changed {
            InsertOutcome::SignaturesAdded
        } else {
            InsertOutcome::Duplicate
        }
    }

    /// Stores an op whose parents are all stored, after checking its lamport clock.
    fn attach(&mut self, op: SignedOp) -> Result<(), LedgerError> {
        let parent_max = op
            .op()
            .parents
            .iter()
            .filter_map(|parent| self.ops.get(parent))
            .map(|parent| parent.op().lamport)
            .max()
            .unwrap_or(0);
        let expected = parent_max
            .checked_add(1)
            .ok_or(LedgerError::LamportOverflow)?;
        if op.op().lamport != expected {
            return Err(LedgerError::WrongLamport {
                expected,
                found: op.op().lamport,
            });
        }
        for parent in &op.op().parents {
            self.heads.remove(parent);
        }
        self.heads.insert(op.id());
        self.ops.insert(op.id(), op);
        Ok(())
    }

    /// An orphan that fails its lamport check is dropped: nobody is waiting for the error.
    fn adopt_orphans(&mut self) {
        loop {
            let ready: Vec<OpId> = self
                .orphans
                .iter()
                .filter(|(_, op)| {
                    op.op()
                        .parents
                        .iter()
                        .all(|parent| self.ops.contains_key(parent))
                })
                .map(|(id, _)| *id)
                .collect();
            if ready.is_empty() {
                return;
            }
            for id in ready {
                if let Some(op) = self.orphans.remove(&id) {
                    let _rejected = self.attach(op);
                }
            }
        }
    }

    /// The ancestors of `op` that are neither stored nor buffered.
    fn missing_ancestors(&self, op: &SignedOp) -> Vec<OpId> {
        let mut missing = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut stack: Vec<OpId> = op.op().parents.clone();
        while let Some(id) = stack.pop() {
            if self.ops.contains_key(&id) || !seen.insert(id) {
                continue;
            }
            match self.orphans.get(&id) {
                Some(orphan) => stack.extend_from_slice(&orphan.op().parents),
                None => {
                    missing.insert(id);
                }
            }
        }
        missing.into_iter().collect()
    }

    fn recompute(&mut self) {
        let heads = self.heads.iter().copied().collect();
        self.view = evaluate(&self.genesis_view, self.founder, &self.ops, heads);
    }
}

pub(crate) const fn network_of(genesis: OpId) -> NetworkId {
    NetworkId::from_bytes(genesis.to_bytes())
}
