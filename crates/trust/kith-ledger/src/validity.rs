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
use kith_types::{AccountId, NodeId, OpId, Timestamp};

use crate::limits::MAX_ADMINS;
use crate::op::Kind;
use crate::view::{Account, Device, Invite, NetworkView, Via};
use crate::{AdminChange, Approval, OpBody, Role, SignedOp, client_id};

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
            | OpBody::RenameDevice { .. }
            | OpBody::Join(_)
            | OpBody::AddAccount { .. }
            | OpBody::RemoveAccount { .. }
            | OpBody::SetRole { .. }
            | OpBody::AssignDevice { .. }
            | OpBody::AuthorizeConsole { .. }
            | OpBody::RevokeConsole { .. }
            | OpBody::SetRecoveryKey { .. }
            | OpBody::Approved { .. }
            | OpBody::Acted { .. }
            | OpBody::Leave => return None,
        }
        Some(next)
    }
}

/// A linear chain of ops of one kind from genesis, and the state after each link.
///
/// Candidates for the link after head H are ops of the kind descending from H that the
/// state at H accepts. Of the earliest candidates (those not descending from another) the
/// lowest `OpId` wins, so every losing branch is invalid.
struct Linear<S> {
    /// Op indices of the links; link 0 is genesis.
    links: Vec<usize>,
    /// `states[k]` is the state after link `k`.
    states: Vec<S>,
    /// For each op, the position of the latest link in its causal past.
    latest_in_past: Vec<usize>,
}

impl<S> Linear<S> {
    /// `step` returns the state after a candidate, or `None` if the state refuses it.
    fn build(
        graph: &Graph<'_>,
        genesis: S,
        kind: Kind,
        mut step: impl FnMut(&S, usize, &SignedOp) -> Option<S>,
    ) -> Self {
        let mut links = vec![0_usize];
        let mut states = vec![genesis];
        while let (Some(&head), Some(state)) = (links.last(), states.last()) {
            let below = graph.descendants(head);
            let mut candidates: Vec<(usize, S)> = Vec::new();
            for (index, op) in graph.indexed() {
                if below.contains(index)
                    && op.op().body.kind() == kind
                    && let Some(next) = step(state, index, op)
                {
                    candidates.push((index, next));
                }
            }
            let indices: Vec<usize> = candidates.iter().map(|(index, _)| *index).collect();
            let winner = candidates
                .iter()
                .enumerate()
                .filter(|(_, (candidate, _))| {
                    indices.len() <= 1 || {
                        let past = graph.ancestors(*candidate);
                        !indices.iter().any(|other| past.contains(*other))
                    }
                })
                .filter_map(|(slot, (index, _))| graph.get(*index).map(|op| (op.id(), slot)))
                .min()
                .map(|(_, slot)| slot);
            let Some((index, next)) = winner.map(|slot| candidates.swap_remove(slot)) else {
                break;
            };
            links.push(index);
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
        Self {
            links,
            states,
            latest_in_past,
        }
    }

    /// The position of the latest link in `index`'s causal past.
    fn latest(&self, index: usize) -> usize {
        self.latest_in_past.get(index).copied().unwrap_or(0)
    }

    /// The state in `index`'s causal past.
    fn state_at(&self, index: usize) -> Option<&S> {
        self.states.get(self.latest(index))
    }

    fn final_state(&self) -> Option<&S> {
        self.states.last()
    }
}

/// A valid `RemoveAdmin` on the winning chain.
struct AdminRemoval {
    admin: NodeId,
    /// Its position in the chain.
    position: usize,
    ancestors: OpSet,
}

/// Stage A's result: the admin devices' authority chain.
struct Chain {
    linear: Linear<Authority>,
    removals: Vec<AdminRemoval>,
}

impl Chain {
    /// Stage A: builds the linear authority chain from genesis.
    fn build(graph: &Graph<'_>, genesis: Authority) -> Self {
        let linear = Linear::build(graph, genesis, Kind::Authority, |state, _, op| {
            if state.authorizes(op) {
                state.apply(&op.op().body)
            } else {
                None
            }
        });
        let removals = linear
            .links
            .iter()
            .enumerate()
            .filter_map(|(link, index)| {
                if let OpBody::RemoveAdmin { admin } = &graph.get(*index)?.op().body {
                    Some(AdminRemoval {
                        admin: *admin,
                        position: link,
                        ancestors: graph.ancestors(*index),
                    })
                } else {
                    None
                }
            })
            .collect();
        Self { linear, removals }
    }

    /// The authority state in `index`'s causal past.
    fn state_at(&self, index: usize) -> Option<&Authority> {
        self.linear.state_at(index)
    }

    /// Rules 1 and 2 for admin ops: `author` is an admin in `index`'s causal past, and no
    /// removal of that admin on the chain is concurrent with `index`.
    fn admin_may_act(&self, index: usize, author: NodeId) -> bool {
        let latest = self.linear.latest(index);
        self.state_at(index)
            .is_some_and(|state| state.admins.contains(&author))
            && self.removals.iter().all(|removal| {
                removal.admin != author
                    || latest >= removal.position
                    || removal.ancestors.contains(index)
            })
    }

    /// Whether `op` is approved as an authority op would be at `index`: its author may act
    /// as an admin, and enough admins signed.
    fn devices_approve(&self, index: usize, op: &SignedOp) -> bool {
        self.admin_may_act(index, op.op().author)
            && self.state_at(index).is_some_and(|state| {
                op.signers()
                    .filter(|signer| state.admins.contains(signer))
                    .count()
                    >= usize::from(state.threshold)
            })
    }

    fn final_state(&self) -> Option<&Authority> {
        self.linear.final_state()
    }
}

/// People, consoles and the recovery key at some point of the account chain.
#[derive(Debug, Clone, Default)]
struct People {
    accounts: BTreeMap<AccountId, Account>,
    consoles: BTreeSet<NodeId>,
    recovery: Option<VerifyingKey>,
}

/// Who approved an account op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Approver {
    /// The admin devices, before the network has any account.
    Devices,
    /// The recovery key.
    Recovery,
    /// A person's passkey.
    Account(Role),
}

impl People {
    fn owners(&self) -> usize {
        self.accounts
            .values()
            .filter(|account| account.role == Role::Owner)
            .count()
    }

    /// The state after `change` approved by `by`, or `None` if it is not allowed.
    fn apply(&self, change: &OpBody, by: Approver, op: OpId) -> Option<Self> {
        let mut next = self.clone();
        match change {
            OpBody::AddAccount {
                name,
                role,
                credential,
            } => {
                let allowed = match by {
                    Approver::Devices => *role == Role::Owner && self.accounts.is_empty(),
                    Approver::Recovery | Approver::Account(Role::Owner) => true,
                    Approver::Account(Role::Admin) => *role != Role::Owner,
                    Approver::Account(Role::Member) => false,
                };
                let account = Account {
                    name: name.clone(),
                    role: *role,
                    credentials: vec![credential.clone()],
                    added_by: op,
                };
                if !allowed
                    || next
                        .accounts
                        .insert(credential.account_id(), account)
                        .is_some()
                {
                    return None;
                }
            }
            OpBody::RemoveAccount { account } => {
                let target = self.accounts.get(account)?.role;
                let allowed = match by {
                    Approver::Account(Role::Owner) => true,
                    Approver::Account(Role::Admin) => target != Role::Owner,
                    Approver::Account(Role::Member) | Approver::Devices | Approver::Recovery => {
                        false
                    }
                };
                if !allowed || (target == Role::Owner && self.owners() <= 1) {
                    return None;
                }
                next.accounts.remove(account);
            }
            OpBody::SetRole { account, role } => {
                let last_owner = self.owners() <= 1;
                let entry = next.accounts.get_mut(account)?;
                if by != Approver::Account(Role::Owner)
                    || entry.role == *role
                    || (entry.role == Role::Owner && last_owner)
                {
                    return None;
                }
                entry.role = *role;
            }
            OpBody::AuthorizeConsole { device } => {
                let allowed = matches!(by, Approver::Account(Role::Owner) | Approver::Recovery);
                if !allowed || !next.consoles.insert(*device) {
                    return None;
                }
            }
            OpBody::RevokeConsole { device } => {
                let allowed = matches!(by, Approver::Account(Role::Owner | Role::Admin));
                if !allowed || !next.consoles.remove(device) {
                    return None;
                }
            }
            OpBody::SetRecoveryKey { key } => {
                if by != Approver::Devices || next.recovery.replace(*key).is_some() {
                    return None;
                }
            }
            OpBody::Genesis { .. }
            | OpBody::AddAdmin { .. }
            | OpBody::RemoveAdmin { .. }
            | OpBody::SetThreshold { .. }
            | OpBody::AddDevice { .. }
            | OpBody::RemoveDevice { .. }
            | OpBody::CreateInvite { .. }
            | OpBody::RevokeInvite { .. }
            | OpBody::SetRecord { .. }
            | OpBody::Join(_)
            | OpBody::AddClient { .. }
            | OpBody::RenameDevice { .. }
            | OpBody::AssignDevice { .. }
            | OpBody::Approved { .. }
            | OpBody::Acted { .. }
            | OpBody::Leave => return None,
        }
        Some(next)
    }
}

/// Approvals already checked, by op index and the key they were checked against: the
/// account chain considers each candidate once per link, and signatures are slow.
#[derive(Default)]
struct Verified(BTreeMap<usize, (Vec<u8>, bool)>);

impl Verified {
    fn check(&mut self, index: usize, key: Vec<u8>, verify: impl FnOnce() -> bool) -> bool {
        if let Some((checked, valid)) = self.0.get(&index)
            && *checked == key
        {
            return *valid;
        }
        let valid = verify();
        self.0.insert(index, (key, valid));
        valid
    }
}

/// Stage A2: the account chain of people, consoles and the recovery key. It depends only
/// on stage A, which admits the first owner and the recovery key.
struct PeopleChain {
    linear: Linear<People>,
    /// The causal past of each link, by position.
    ancestors: Vec<OpSet>,
}

impl PeopleChain {
    fn build(graph: &Graph<'_>, chain: &Chain) -> Self {
        let mut verified = Verified::default();
        let linear = Linear::build(
            graph,
            People::default(),
            Kind::Account,
            |state, index, op| people_step(state, index, op, chain, &mut verified),
        );
        let ancestors = linear
            .links
            .iter()
            .map(|link| graph.ancestors(*link))
            .collect();
        Self { linear, ancestors }
    }

    /// Whether `holds` is true of the state in `index`'s causal past and after every link
    /// concurrent with `index`: a concurrent removal wins.
    fn holds(&self, index: usize, holds: impl Fn(&People) -> bool) -> bool {
        let latest = self.linear.latest(index);
        self.linear.state_at(index).is_some_and(&holds)
            && self
                .linear
                .states
                .iter()
                .zip(&self.ancestors)
                .skip(latest.saturating_add(1))
                .all(|(state, past)| past.contains(index) || holds(state))
    }

    /// Whether `actor` may make everyday admin changes in an op at `index`.
    fn may_act(&self, index: usize, actor: &AccountId) -> bool {
        self.holds(index, |people| {
            people
                .accounts
                .get(actor)
                .is_some_and(|account| account.role.can_administer())
        })
    }

    /// Whether `actor` is a person in the network for an op at `index`, in any role.
    fn is_person(&self, index: usize, actor: &AccountId) -> bool {
        self.holds(index, |people| people.accounts.contains_key(actor))
    }

    /// Whether `device` is an authorized console for an op at `index`.
    fn console_may_act(&self, index: usize, device: &NodeId) -> bool {
        self.holds(index, |people| people.consoles.contains(device))
    }

    /// Whether `account` exists in `index`'s causal past.
    fn exists_at(&self, index: usize, account: &AccountId) -> bool {
        self.linear
            .state_at(index)
            .is_some_and(|people| people.accounts.contains_key(account))
    }
}

/// The state after account op `op`, or `None` if `state` does not accept it.
fn people_step(
    state: &People,
    index: usize,
    op: &SignedOp,
    chain: &Chain,
    verified: &mut Verified,
) -> Option<People> {
    let body = &op.op().body;
    if let OpBody::Approved { approval, .. } = body {
        let challenge = op.challenge()?;
        let by = match approval {
            Approval::Passkey(assertion) => {
                let account = state.accounts.get(&assertion.account)?;
                let credential = account
                    .credentials
                    .iter()
                    .find(|credential| credential.id == assertion.credential_id)?;
                let mut key = credential.public_key.clone();
                key.extend_from_slice(credential.rp_id.as_bytes());
                let valid = verified.check(index, key, || {
                    assertion.verify(credential, &challenge).is_ok()
                });
                valid.then_some(Approver::Account(account.role))?
            }
            Approval::Recovery(signature) => {
                let key = state.recovery.as_ref()?;
                let valid = verified.check(index, key.to_bytes().to_vec(), || {
                    Approval::verify_recovery(key, &challenge, signature)
                });
                valid.then_some(Approver::Recovery)?
            }
        };
        return state.apply(op.effect(), by, op.id());
    }
    let device_path = matches!(
        body,
        OpBody::AddAccount { .. } | OpBody::SetRecoveryKey { .. }
    );
    if device_path && state.accounts.is_empty() && chain.devices_approve(index, op) {
        return state.apply(body, Approver::Devices, op.id());
    }
    None
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
    created: Timestamp,
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
    /// Valid `RenameDevice` ops, in order: (index, device, name).
    renames: Vec<(usize, NodeId, &'a String)>,
    /// Tags granted to each valid join, from its invite.
    joined_tags: BTreeMap<usize, &'a [String]>,
    /// Valid `AssignDevice` ops, in order: (index, device, account).
    assignments: Vec<(usize, NodeId, AccountId)>,
}

impl<'a> StageB<'a> {
    fn removals_of(&self, device: &NodeId) -> &[Removal] {
        self.device_removals.get(device).map_or(&[], Vec::as_slice)
    }

    /// Admin ops depend only on stages A and A2, so their validity is settled in one pass.
    /// An op made for a person also needs that person to hold an admin role, and may come
    /// from an authorized console instead of an admin device.
    fn admin_ops(&mut self, graph: &Graph<'a>, chain: &Chain, people: &PeopleChain) {
        for (index, op) in graph.indexed() {
            let author = op.op().author;
            if matches!(op.op().body, OpBody::Leave) {
                // A device's own signature is all a removal of itself needs.
                self.device_removals
                    .entry(author)
                    .or_default()
                    .push(Removal::new(graph, index));
                continue;
            }
            let valid = if let OpBody::Acted { actor, change } = &op.op().body {
                ((chain.admin_may_act(index, author) || people.console_may_act(index, &author))
                    && people.may_act(index, actor))
                    || (own_device_invite(change)
                        && people.console_may_act(index, &author)
                        && people.is_person(index, actor))
            } else {
                op.op().body.kind() == Kind::Admin && chain.admin_may_act(index, author)
            };
            if !valid {
                continue;
            }
            match op.effect() {
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
                            created: op.op().timestamp,
                        });
                }
                OpBody::RevokeInvite { invite_key } => self
                    .revocations
                    .entry(invite_key.to_bytes())
                    .or_default()
                    .push(Removal::new(graph, index)),
                OpBody::SetRecord { key, value } => self.set_records.push((key, value)),
                OpBody::RenameDevice { device, name } => {
                    self.renames.push((index, *device, name));
                }
                OpBody::AssignDevice { device, account } => {
                    if people.exists_at(index, account) {
                        self.assignments.push((index, *device, *account));
                    }
                }
                OpBody::Genesis { .. }
                | OpBody::AddAdmin { .. }
                | OpBody::RemoveAdmin { .. }
                | OpBody::SetThreshold { .. }
                | OpBody::Join(_)
                | OpBody::AddAccount { .. }
                | OpBody::RemoveAccount { .. }
                | OpBody::SetRole { .. }
                | OpBody::AuthorizeConsole { .. }
                | OpBody::RevokeConsole { .. }
                | OpBody::SetRecoveryKey { .. }
                | OpBody::Approved { .. }
                | OpBody::Acted { .. }
                | OpBody::Leave => {}
            }
        }
    }

    /// A `RemoveAccount` on the account chain removes, like a `RemoveDevice` at the same
    /// point, every device last assigned to the account in its causal past.
    fn account_removals(&mut self, graph: &Graph<'a>, people: &PeopleChain) {
        for (link, past) in people.linear.links.iter().zip(&people.ancestors) {
            let Some(OpBody::RemoveAccount { account }) = graph.get(*link).map(SignedOp::effect)
            else {
                continue;
            };
            let mut owners: BTreeMap<NodeId, AccountId> = BTreeMap::new();
            for (index, device, owner) in &self.assignments {
                if past.contains(*index) {
                    owners.insert(*device, *owner);
                }
            }
            for (device, owner) in owners {
                if owner == *account {
                    self.device_removals
                        .entry(device)
                        .or_default()
                        .push(Removal::new(graph, *link));
                }
            }
        }
    }

    /// Whose each device is: the latest valid assignment after its admitting add, to an
    /// account that still exists.
    fn owners(
        &self,
        admitted: &BTreeMap<NodeId, usize>,
        accounts: &BTreeMap<AccountId, Account>,
    ) -> BTreeMap<NodeId, AccountId> {
        let mut owners = BTreeMap::new();
        for (index, device, account) in &self.assignments {
            if admitted.get(device).is_some_and(|add| index > add) {
                owners.insert(*device, *account);
            }
        }
        owners.retain(|_, account| accounts.contains_key(account));
        owners
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
    /// Also returns the index of each member's admitting add.
    fn devices(&self, graph: &Graph<'a>) -> (BTreeMap<NodeId, Device>, BTreeMap<NodeId, usize>) {
        let mut devices = BTreeMap::new();
        let mut admitted = BTreeMap::new();
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
            let (name, tags, via) = match op.effect() {
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
                | OpBody::SetRecord { .. }
                | OpBody::RenameDevice { .. }
                | OpBody::AddAccount { .. }
                | OpBody::RemoveAccount { .. }
                | OpBody::SetRole { .. }
                | OpBody::AssignDevice { .. }
                | OpBody::AuthorizeConsole { .. }
                | OpBody::RevokeConsole { .. }
                | OpBody::SetRecoveryKey { .. }
                | OpBody::Approved { .. }
                | OpBody::Acted { .. }
                | OpBody::Leave => continue,
            };
            // Last writer wins among renames ordered after the admitting add.
            let name = self
                .renames
                .iter()
                .rev()
                .find(|(index, target, _)| target == device && *index > add)
                .map_or(name, |(_, _, renamed)| (*renamed).clone());
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
            admitted.insert(*device, add);
        }
        (devices, admitted)
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
                    created: invite.created,
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
    let people = PeopleChain::build(&graph, &chain);
    let mut stage = StageB::default();
    stage.adds.entry(founder).or_default().push(0);
    stage.admin_ops(&graph, &chain, &people);
    stage.account_removals(&graph, &people);
    stage.joins(&graph);

    let mut view = genesis_view.clone();
    if let Some(state) = chain.final_state() {
        view.admins.clone_from(&state.admins);
        view.threshold = state.threshold;
    }
    if let Some(state) = people.linear.final_state() {
        view.accounts.clone_from(&state.accounts);
        view.consoles.clone_from(&state.consoles);
        view.recovery_key = state.recovery;
    }
    let (devices, admitted) = stage.devices(&graph);
    view.device_owners = stage.owners(&admitted, &view.accounts);
    view.devices = devices;
    view.invites = stage.invites();
    view.records = stage.records();
    view.account_changes = people
        .linear
        .links
        .iter()
        .skip(1)
        .filter_map(|index| graph.get(*index).map(SignedOp::id))
        .collect();
    view.heads = heads;
    view
}

/// Whether `change` is an invite a console may make for any person adding their own
/// device: single use, with no tags.
fn own_device_invite(change: &AdminChange) -> bool {
    matches!(
        change,
        AdminChange::CreateInvite { max_uses: 1, tags, .. } if tags.is_empty()
    )
}
