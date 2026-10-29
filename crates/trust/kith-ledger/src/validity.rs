//! Deciding which stored ops are valid, and deriving the [`NetworkView`] from them.
//!
//! A pure function of the stored ops, implementing the rules in the crate docs. Ops are
//! indexed by `(lamport, OpId)` order, which is topological since lamport strictly
//! increases along every edge; genesis is index 0.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use kith_crypto::VerifyingKey;
use kith_types::{NodeId, OpId, Timestamp};

use crate::limits::MAX_ADMINS;
use crate::op::Kind;
use crate::view::{Device, Invite, NetworkView, Via};
use crate::{OpBody, SignedOp, client_id};

/// A set of op indices, as a bitmap: `n` ops take `n / 8` bytes.
struct OpSet(Vec<u64>);

impl OpSet {
    fn new(len: usize) -> Self {
        Self(vec![0; len.div_ceil(64)])
    }

    /// Adds `index`; returns whether it was absent.
    fn insert(&mut self, index: usize) -> bool {
        let (word, mask) = locate(index);
        self.0.get_mut(word).is_some_and(|bits| {
            let absent = *bits & mask == 0;
            *bits |= mask;
            absent
        })
    }

    fn contains(&self, index: usize) -> bool {
        let (word, mask) = locate(index);
        self.0.get(word).is_some_and(|bits| bits & mask != 0)
    }
}

const fn locate(index: usize) -> (usize, u64) {
    (index / 64, 1 << (index % 64))
}

/// The causal DAG over stored ops, in topological order.
struct Graph<'a> {
    ops: Vec<&'a SignedOp>,
    parents: Vec<Vec<usize>>,
    children: Vec<Vec<usize>>,
}

impl<'a> Graph<'a> {
    fn new(ops: &'a BTreeMap<OpId, SignedOp>) -> Self {
        let mut sorted: Vec<&SignedOp> = ops.values().collect();
        sorted.sort_by_key(|op| (op.op().lamport, op.id()));
        let index: BTreeMap<OpId, usize> = sorted
            .iter()
            .enumerate()
            .map(|(position, op)| (op.id(), position))
            .collect();
        // The ledger only stores ops whose parents are stored, so no parent is skipped.
        let parents: Vec<Vec<usize>> = sorted
            .iter()
            .map(|op| {
                op.op()
                    .parents
                    .iter()
                    .filter_map(|parent| index.get(parent).copied())
                    .collect()
            })
            .collect();
        let mut children = vec![Vec::new(); sorted.len()];
        for (child, parent_list) in parents.iter().enumerate() {
            for parent in parent_list {
                if let Some(list) = children.get_mut(*parent) {
                    list.push(child);
                }
            }
        }
        Self {
            ops: sorted,
            parents,
            children,
        }
    }

    fn len(&self) -> usize {
        self.ops.len()
    }

    /// Strict ancestors of `start`: its causal past.
    fn ancestors(&self, start: usize) -> OpSet {
        walk(&self.parents, start)
    }

    /// Strict descendants of `start`: every op that has it in its causal past.
    fn descendants(&self, start: usize) -> OpSet {
        walk(&self.children, start)
    }

    fn indexed(&self) -> impl Iterator<Item = (usize, &'a SignedOp)> + '_ {
        self.ops.iter().copied().enumerate()
    }

    fn get(&self, index: usize) -> Option<&'a SignedOp> {
        self.ops.get(index).copied()
    }
}

fn walk(edges: &[Vec<usize>], start: usize) -> OpSet {
    let mut seen = OpSet::new(edges.len());
    let mut stack: Vec<usize> = edges.get(start).cloned().unwrap_or_default();
    while let Some(index) = stack.pop() {
        if seen.insert(index)
            && let Some(next) = edges.get(index)
        {
            stack.extend_from_slice(next);
        }
    }
    seen
}

/// The admin set and threshold at some point of the authority chain.
#[derive(Debug, Clone)]
struct Authority {
    admins: BTreeSet<NodeId>,
    threshold: u8,
}

impl Authority {
    /// Whether this state accepts authority op `op` as its next chain link: the author is
    /// an admin, enough distinct admins signed, and the change itself is allowed.
    fn authorizes(&self, op: &SignedOp) -> bool {
        let approvals = op
            .signers()
            .filter(|signer| self.admins.contains(signer))
            .count();
        self.admins.contains(&op.op().author)
            && approvals >= usize::from(self.threshold)
            && self.apply(&op.op().body).is_some()
    }

    /// The state after `body`, or `None` if the change is not allowed from this state.
    fn apply(&self, body: &OpBody) -> Option<Self> {
        let mut next = self.clone();
        match body {
            OpBody::AddAdmin { admin } => {
                if next.admins.len() >= MAX_ADMINS || !next.admins.insert(*admin) {
                    return None;
                }
            }
            OpBody::RemoveAdmin { admin } => {
                if !next.admins.remove(admin) || next.admins.len() < usize::from(next.threshold) {
                    return None;
                }
            }
            OpBody::SetThreshold { threshold } => {
                if *threshold == 0 || usize::from(*threshold) > next.admins.len() {
                    return None;
                }
                next.threshold = *threshold;
            }
            OpBody::Genesis { .. }
            | OpBody::AddDevice { .. }
            | OpBody::RemoveDevice { .. }
            | OpBody::CreateInvite { .. }
            | OpBody::RevokeInvite { .. }
            | OpBody::SetRecord { .. }
            | OpBody::AddClient { .. }
            | OpBody::Join(_) => return None,
        }
        Some(next)
    }
}

/// A valid `RemoveAdmin` on the winning chain.
struct AdminRemoval {
    admin: NodeId,
    /// Its position in the chain.
    position: usize,
    ancestors: OpSet,
}

/// Stage A's result.
struct Chain {
    /// `states[k]` is the authority state after chain link `k`; link 0 is genesis.
    states: Vec<Authority>,
    /// For each op, the position of the latest chain link in its causal past.
    latest_in_past: Vec<usize>,
    removals: Vec<AdminRemoval>,
}

impl Chain {
    /// Stage A: builds the linear authority chain from genesis.
    fn build(graph: &Graph<'_>, genesis: Authority) -> Self {
        let mut links = vec![0_usize];
        let mut states = vec![genesis];
        while let (Some(&head), Some(state)) = (links.last(), states.last()) {
            let below = graph.descendants(head);
            let candidates: Vec<usize> = graph
                .indexed()
                .filter(|(index, op)| {
                    below.contains(*index)
                        && op.op().body.kind() == Kind::Authority
                        && state.authorizes(op)
                })
                .map(|(index, _)| index)
                .collect();
            let direct: Vec<usize> = if candidates.len() <= 1 {
                candidates
            } else {
                candidates
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        let past = graph.ancestors(*candidate);
                        !candidates.iter().any(|other| past.contains(*other))
                    })
                    .collect()
            };
            let winner = direct
                .into_iter()
                .filter_map(|index| graph.get(index).map(|op| (op.id(), index)))
                .min();
            let Some((_, winner)) = winner else { break };
            let Some(next) = graph.get(winner).and_then(|op| state.apply(&op.op().body)) else {
                break;
            };
            links.push(winner);
            states.push(next);
        }

        let mut position = vec![None; graph.len()];
        for (link, index) in links.iter().enumerate() {
            if let Some(slot) = position.get_mut(*index) {
                *slot = Some(link);
            }
        }
        // Topological order means every parent is final before its children are visited.
        let mut latest_in_past = vec![0_usize; graph.len()];
        for index in 0..graph.len() {
            let latest = graph
                .parents
                .get(index)
                .into_iter()
                .flatten()
                .map(|parent| {
                    let inherited = latest_in_past.get(*parent).copied().unwrap_or(0);
                    let own = position.get(*parent).copied().flatten().unwrap_or(0);
                    inherited.max(own)
                })
                .max()
                .unwrap_or(0);
            if let Some(slot) = latest_in_past.get_mut(index) {
                *slot = latest;
            }
        }

        let removals = links
            .iter()
            .enumerate()
            .filter_map(|(link, index)| match &graph.get(*index)?.op().body {
                OpBody::RemoveAdmin { admin } => Some(AdminRemoval {
                    admin: *admin,
                    position: link,
                    ancestors: graph.ancestors(*index),
                }),
                OpBody::Genesis { .. }
                | OpBody::AddAdmin { .. }
                | OpBody::SetThreshold { .. }
                | OpBody::AddDevice { .. }
                | OpBody::RemoveDevice { .. }
                | OpBody::CreateInvite { .. }
                | OpBody::RevokeInvite { .. }
                | OpBody::SetRecord { .. }
                | OpBody::AddClient { .. }
                | OpBody::Join(_) => None,
            })
            .collect();

        Self {
            states,
            latest_in_past,
            removals,
        }
    }

    /// The authority state in `index`'s causal past.
    fn state_at(&self, index: usize) -> Option<&Authority> {
        let link = self.latest_in_past.get(index).copied()?;
        self.states.get(link)
    }

    /// Rules 1 and 2 for admin ops: `author` is an admin in `index`'s causal past, and no
    /// removal of that admin on the chain is concurrent with `index`.
    fn admin_may_act(&self, index: usize, author: NodeId) -> bool {
        let latest = self.latest_in_past.get(index).copied().unwrap_or(0);
        self.state_at(index)
            .is_some_and(|state| state.admins.contains(&author))
            && self.removals.iter().all(|removal| {
                removal.admin != author
                    || latest >= removal.position
                    || removal.ancestors.contains(index)
            })
    }

    fn final_state(&self) -> Option<&Authority> {
        self.states.last()
    }
}

/// A valid `RemoveDevice` or `RevokeInvite`, with what is needed to compare it causally.
struct Removal {
    /// Ops in its causal past: those it has "seen".
    ancestors: OpSet,
    /// Ops that have it in their causal past.
    descendants: OpSet,
}

impl Removal {
    fn new(graph: &Graph<'_>, index: usize) -> Self {
        Self {
            ancestors: graph.ancestors(index),
            descendants: graph.descendants(index),
        }
    }

    /// Whether this removal is strictly in `index`'s causal past.
    fn precedes(&self, index: usize) -> bool {
        self.descendants.contains(index)
    }

    /// Whether `index` is strictly in this removal's causal past.
    fn follows(&self, index: usize) -> bool {
        self.ancestors.contains(index)
    }
}

/// The first valid `CreateInvite` for a key, and how often it has been redeemed.
struct InviteState<'a> {
    create: usize,
    key: VerifyingKey,
    expires: Timestamp,
    max_uses: u16,
    tags: &'a [String],
    uses: u16,
}

/// Stage B's working state.
#[derive(Default)]
struct StageB<'a> {
    /// Valid `RemoveDevice` ops, by device.
    device_removals: BTreeMap<NodeId, Vec<Removal>>,
    /// Valid `RevokeInvite` ops, by invite key.
    revocations: BTreeMap<[u8; 32], Vec<Removal>>,
    /// The governing `CreateInvite` per invite key.
    invites: BTreeMap<[u8; 32], InviteState<'a>>,
    /// Valid adds (genesis, `AddDevice`, `Join`), by device, ascending.
    adds: BTreeMap<NodeId, Vec<usize>>,
    /// Valid `SetRecord` ops, in order.
    set_records: Vec<(&'a String, &'a Option<Vec<u8>>)>,
    /// Tags granted to each valid join, from its invite.
    joined_tags: BTreeMap<usize, &'a [String]>,
}

impl<'a> StageB<'a> {
    fn removals_of(&self, device: &NodeId) -> &[Removal] {
        self.device_removals.get(device).map_or(&[], Vec::as_slice)
    }

    /// Admin ops depend only on stage A, so their validity is settled in one pass.
    fn admin_ops(&mut self, graph: &Graph<'a>, chain: &Chain) {
        for (index, op) in graph.indexed() {
            if op.op().body.kind() != Kind::Admin || !chain.admin_may_act(index, op.op().author) {
                continue;
            }
            match &op.op().body {
                OpBody::AddDevice { device, .. } => {
                    self.adds.entry(*device).or_default().push(index);
                }
                OpBody::AddClient { key, .. } => {
                    self.adds.entry(client_id(key)).or_default().push(index);
                }
                OpBody::RemoveDevice { device } => self
                    .device_removals
                    .entry(*device)
                    .or_default()
                    .push(Removal::new(graph, index)),
                OpBody::CreateInvite {
                    invite_key,
                    expires,
                    max_uses,
                    tags,
                } => {
                    // Indices ascend, so the first create seen for a key governs it.
                    self.invites
                        .entry(invite_key.to_bytes())
                        .or_insert_with(|| InviteState {
                            create: index,
                            key: *invite_key,
                            expires: *expires,
                            max_uses: *max_uses,
                            tags,
                            uses: 0,
                        });
                }
                OpBody::RevokeInvite { invite_key } => self
                    .revocations
                    .entry(invite_key.to_bytes())
                    .or_default()
                    .push(Removal::new(graph, index)),
                OpBody::SetRecord { key, value } => self.set_records.push((key, value)),
                OpBody::Genesis { .. }
                | OpBody::AddAdmin { .. }
                | OpBody::RemoveAdmin { .. }
                | OpBody::SetThreshold { .. }
                | OpBody::Join(_) => {}
            }
        }
    }

    /// Joins depend on earlier joins (a sponsor may have joined itself) and on invite use
    /// counts, so they are decided one by one in topological order.
    fn joins(&mut self, graph: &Graph<'a>) {
        for (index, op) in graph.indexed() {
            let OpBody::Join(proof) = &op.op().body else {
                continue;
            };
            let past = graph.ancestors(index);
            let sponsor = op.op().author;
            let sponsor_removals = self.removals_of(&sponsor);
            // Rules 1 and 2: some add of the sponsor is in the join's past, and every
            // removal of the sponsor either precedes that add or comes after the join.
            let sponsor_ok = self.adds.get(&sponsor).is_some_and(|list| {
                list.iter().any(|add| {
                    past.contains(*add)
                        && sponsor_removals
                            .iter()
                            .all(|removal| removal.precedes(*add) || removal.follows(index))
                })
            });
            let key = proof.invite_key.to_bytes();
            let revoke_ok = self
                .revocations
                .get(&key)
                .is_none_or(|list| list.iter().all(|removal| removal.follows(index)));
            let Some(invite) = self.invites.get_mut(&key) else {
                continue;
            };
            let invite_ok = past.contains(invite.create)
                && op.op().timestamp <= invite.expires
                && invite.uses < invite.max_uses;
            if sponsor_ok && revoke_ok && invite_ok {
                invite.uses = invite.uses.saturating_add(1);
                self.adds.entry(proof.device).or_default().push(index);
                self.joined_tags.insert(index, invite.tags);
            }
        }
    }

    /// A device is a member if some valid add of it has every valid removal of it in its
    /// causal past. After a re-add, the latest such add describes the device.
    fn devices(&self, graph: &Graph<'a>) -> BTreeMap<NodeId, Device> {
        let mut devices = BTreeMap::new();
        for (device, list) in &self.adds {
            let removals = self.removals_of(device);
            let admitting = list
                .iter()
                .copied()
                .filter(|add| removals.iter().all(|removal| removal.precedes(*add)))
                .max();
            let Some((add, op)) = admitting.and_then(|add| Some((add, graph.get(add)?))) else {
                continue;
            };
            let (name, tags, via) = match &op.op().body {
                OpBody::Genesis { founder_name, .. } => (founder_name.clone(), Vec::new(), None),
                OpBody::AddDevice { name, tags, .. } => (name.clone(), tags.clone(), None),
                OpBody::AddClient {
                    key,
                    gateway,
                    name,
                    tags,
                    v4,
                    v6,
                } => (
                    name.clone(),
                    tags.clone(),
                    Some(Via {
                        gateway: *gateway,
                        key: *key,
                        v4: *v4,
                        v6: *v6,
                    }),
                ),
                OpBody::Join(proof) => (
                    proof.name.clone(),
                    self.joined_tags
                        .get(&add)
                        .map_or_else(Vec::new, |tags| tags.to_vec()),
                    None,
                ),
                OpBody::AddAdmin { .. }
                | OpBody::RemoveAdmin { .. }
                | OpBody::SetThreshold { .. }
                | OpBody::RemoveDevice { .. }
                | OpBody::CreateInvite { .. }
                | OpBody::RevokeInvite { .. }
                | OpBody::SetRecord { .. } => continue,
            };
            devices.insert(
                *device,
                Device {
                    name,
                    tags,
                    admitted_by: op.id(),
                    admitted_at_lamport: op.op().lamport,
                    via,
                },
            );
        }
        devices
    }

    /// Invites neither revoked (revocation is permanent, whatever its causal relation to
    /// the create) nor used up.
    fn invites(&self) -> BTreeMap<[u8; 32], Invite> {
        self.invites
            .iter()
            .filter(|(key, invite)| {
                !self.revocations.contains_key(*key) && invite.uses < invite.max_uses
            })
            .map(|(key, invite)| {
                let entry = Invite {
                    key: invite.key,
                    expires: invite.expires,
                    max_uses: invite.max_uses,
                    uses: invite.uses,
                    tags: invite.tags.to_vec(),
                };
                (*key, entry)
            })
            .collect()
    }

    /// Last writer wins: `set_records` is already in `(lamport, OpId)` order.
    fn records(&self) -> BTreeMap<String, Vec<u8>> {
        let mut records = BTreeMap::new();
        for (key, value) in &self.set_records {
            match value {
                Some(value) => {
                    records.insert((*key).clone(), value.clone());
                }
                None => {
                    records.remove(*key);
                }
            }
        }
        records
    }
}

/// Derives the view of the stored ops. `ops` must contain genesis and every op's parents.
pub(crate) fn evaluate(
    genesis_view: &NetworkView,
    founder: NodeId,
    ops: &BTreeMap<OpId, SignedOp>,
    heads: Vec<OpId>,
) -> NetworkView {
    let graph = Graph::new(ops);
    let chain = Chain::build(
        &graph,
        Authority {
            admins: genesis_view.admins.clone(),
            threshold: genesis_view.threshold,
        },
    );
    let mut stage = StageB::default();
    stage.adds.entry(founder).or_default().push(0);
    stage.admin_ops(&graph, &chain);
    stage.joins(&graph);

    let mut view = genesis_view.clone();
    if let Some(state) = chain.final_state() {
        view.admins.clone_from(&state.admins);
        view.threshold = state.threshold;
    }
    view.devices = stage.devices(&graph);
    view.invites = stage.invites();
    view.records = stage.records();
    view.heads = heads;
    view
}
