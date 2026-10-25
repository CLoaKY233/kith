//! Membership replication: ledgers synced by [`SyncSession`]s over a chaotic network.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use kith_crypto::SigningKey;
use kith_ledger::{JoinProof, Ledger, NetworkParams, OpBody};
use kith_sync::SyncSession;
use kith_types::{NodeId, OpId, Timestamp};
use proptest::prelude::*;

use super::config;
use crate::{Delivery, HostId, LinkConfig, Sim};

const PORT: u16 = 7000;
const TICK: Duration = Duration::from_millis(250);
const SESSION_IDLE: Duration = Duration::from_secs(3);
const HEADER: usize = 13;

fn key(seed: u64) -> SigningKey {
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    bytes[31] = 0x5a;
    SigningKey::from_seed(&bytes)
}

/// One side of a sync session on an unreliable network: messages are numbered, reordered
/// ones are buffered, and a session that loses one stalls until it idles out.
struct Session {
    initiator: bool,
    sync: SyncSession,
    next_out: u32,
    next_in: u32,
    early: BTreeMap<u32, Vec<u8>>,
    last_heard: Duration,
}

struct Device {
    key: SigningKey,
    host: HostId,
    ledger: Ledger,
    sessions: BTreeMap<(HostId, u64), Session>,
    next_session: u64,
}

struct Cluster {
    sim: Sim,
    devices: Vec<Device>,
    invites: Vec<SigningKey>,
    fresh_keys: u64,
}

impl Cluster {
    /// `count` replicas of one network. Device 0 founded it, device 1 is a second admin,
    /// and the rest are members, half added directly and half joined by invite.
    fn new(seed: u64, count: usize, link: LinkConfig) -> Self {
        let mut sim = Sim::new(seed);
        sim.set_link(link);
        let params = NetworkParams {
            ipv4: "100.64.0.0/10".parse().unwrap(),
            ipv6: "fd7a:115c:a1e0::/48".parse().unwrap(),
        };
        let keys: Vec<SigningKey> = (0..u64::try_from(count).unwrap()).map(key).collect();
        let founder = &keys[0];
        let at = Timestamp::from_millis(1);
        let mut ledger = Ledger::create(founder, "home", "device-0", params, &[], 1, at).unwrap();
        let mut cluster_invites = Vec::new();
        let invite = key(1_000);
        let author = |ledger: &mut Ledger, body| {
            let op = ledger
                .draft(founder.node_id(), body, at)
                .sign(founder)
                .unwrap();
            ledger.insert(op).unwrap();
        };
        author(
            &mut ledger,
            OpBody::CreateInvite {
                invite_key: invite.verifying_key(),
                expires: Timestamp::from_millis(u64::MAX),
                max_uses: 1_000,
                tags: Vec::new(),
            },
        );
        for (index, device) in keys.iter().enumerate().skip(1) {
            let name = format!("device-{index}");
            if index % 2 == 0 {
                let proof = JoinProof::create(ledger.network_id(), &invite, device, &name).unwrap();
                author(&mut ledger, OpBody::Join(proof));
            } else {
                author(
                    &mut ledger,
                    OpBody::AddDevice {
                        device: device.node_id(),
                        name,
                        tags: Vec::new(),
                    },
                );
            }
        }
        author(
            &mut ledger,
            OpBody::AddAdmin {
                admin: keys[1].node_id(),
            },
        );
        cluster_invites.push(invite);
        let devices = keys
            .into_iter()
            .map(|key| Device {
                key,
                host: sim.add_host(None),
                ledger: ledger.clone(),
                sessions: BTreeMap::new(),
                next_session: 0,
            })
            .collect();
        Self {
            sim,
            devices,
            invites: cluster_invites,
            fresh_keys: 2_000,
        }
    }

    fn hosts(&self, indices: &[usize]) -> Vec<HostId> {
        indices
            .iter()
            .map(|index| self.devices[*index].host)
            .collect()
    }

    fn fresh_key(&mut self) -> SigningKey {
        self.fresh_keys += 1;
        key(self.fresh_keys)
    }

    /// Device `index` drafts, signs and stores an op on its own replica.
    fn act(&mut self, index: usize, body: OpBody) -> OpId {
        let millis = u64::try_from(self.sim.now().as_millis()).unwrap();
        let device = &mut self.devices[index];
        let op = device
            .ledger
            .draft(
                device.key.node_id(),
                body,
                Timestamp::from_millis(millis + 2),
            )
            .sign(&device.key)
            .unwrap();
        let id = op.id();
        device.ledger.insert(op).unwrap();
        id
    }

    fn join(&mut self, sponsor: usize, invite: usize) -> NodeId {
        let joiner = self.fresh_key();
        let network = self.devices[sponsor].ledger.network_id();
        let proof = JoinProof::create(network, &self.invites[invite], &joiner, "guest").unwrap();
        self.act(sponsor, OpBody::Join(proof));
        joiner.node_id()
    }

    /// One random ledger change by a random device, authorized or not.
    fn random_change(&mut self) {
        let count = self.devices.len();
        let author = self.sim.rng().index(count);
        let target = self.devices[1 + self.sim.rng().index(count - 1)]
            .key
            .node_id();
        let value = self.sim.rng().next_u64().to_le_bytes().to_vec();
        let body = match self.sim.rng().below(6) {
            0 | 1 => OpBody::SetRecord {
                key: format!("key-{}", value[0] % 4),
                value: (!value[1].is_multiple_of(4)).then_some(value),
            },
            2 => OpBody::RemoveDevice { device: target },
            3 => OpBody::AddDevice {
                device: target,
                name: String::from("re-added"),
                tags: Vec::new(),
            },
            4 => {
                let invite = self.fresh_key();
                let body = OpBody::CreateInvite {
                    invite_key: invite.verifying_key(),
                    expires: Timestamp::from_millis(u64::MAX),
                    max_uses: 2,
                    tags: vec![String::from("guest")],
                };
                self.invites.push(invite);
                body
            }
            _ => {
                let invite = self.sim.rng().index(self.invites.len());
                self.join(author, invite);
                return;
            }
        };
        self.act(author, body);
    }

    /// Starts a sync session from `index` to a random peer, unless it already has two.
    fn start_session(&mut self, index: usize) {
        let peer = self.sim.rng().index(self.devices.len());
        let peer_host = self.devices[peer].host;
        let device = &mut self.devices[index];
        let outgoing = device.sessions.values().filter(|s| s.initiator).count();
        if peer == index || outgoing >= 2 {
            return;
        }
        let id = (u64::try_from(index).unwrap() << 32) | device.next_session;
        device.next_session += 1;
        let session = Session {
            initiator: true,
            sync: SyncSession::new(true, device.ledger.network_id()),
            next_out: 0,
            next_in: 0,
            early: BTreeMap::new(),
            last_heard: self.sim.now(),
        };
        device.sessions.insert((peer_host, id), session);
        self.pump(index);
    }

    /// Sends everything device `index`'s sessions have queued, and retires finished or
    /// idle sessions.
    fn pump(&mut self, index: usize) {
        let now = self.sim.now();
        let device = &mut self.devices[index];
        for (&(peer, id), session) in &mut device.sessions {
            while let Some(message) = session.sync.poll_transmit(&device.ledger) {
                let mut packet = Vec::with_capacity(HEADER + message.len());
                packet.push(u8::from(session.initiator));
                packet.extend_from_slice(&id.to_le_bytes());
                packet.extend_from_slice(&session.next_out.to_le_bytes());
                packet.extend_from_slice(&message);
                session.next_out += 1;
                let to = self.sim.address(peer, PORT).unwrap();
                self.sim.send_datagram(device.host, PORT, to, &packet);
            }
        }
        device.sessions.retain(|_, session| {
            !session.sync.is_done() && now.saturating_sub(session.last_heard) < SESSION_IDLE
        });
    }

    fn on_datagram(&mut self, host: HostId, from: HostId, packet: &[u8]) {
        let Some(index) = self.devices.iter().position(|device| device.host == host) else {
            return;
        };
        let (header, message) = packet.split_at(HEADER);
        let from_initiator = header[0] == 1;
        let id = u64::from_le_bytes(header[1..9].try_into().unwrap());
        let seq = u32::from_le_bytes(header[9..13].try_into().unwrap());
        let now = self.sim.now();
        let device = &mut self.devices[index];
        let network = device.ledger.network_id();
        let session = device.sessions.entry((from, id));
        let session = match session {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) if from_initiator => {
                entry.insert(Session {
                    initiator: false,
                    sync: SyncSession::new(false, network),
                    next_out: 0,
                    next_in: 0,
                    early: BTreeMap::new(),
                    last_heard: now,
                })
            }
            std::collections::btree_map::Entry::Vacant(_) => return,
        };
        if seq >= session.next_in {
            session.early.insert(seq, message.to_vec());
        }
        session.last_heard = now;
        let mut failed = false;
        while let Some(message) = session.early.remove(&session.next_in) {
            session.next_in += 1;
            if session
                .sync
                .handle_message(&mut device.ledger, &message)
                .is_err()
            {
                failed = true;
                break;
            }
        }
        if failed {
            device.sessions.remove(&(from, id));
        }
        self.pump(index);
    }

    /// Runs for `duration`: every tick each device may change its ledger (with probability
    /// `change_per_mille`) and may start a sync.
    fn run(&mut self, duration: Duration, change_per_mille: u16) {
        let end = self.sim.now() + duration;
        while self.sim.now() < end {
            let tick_end = self.sim.now() + TICK;
            while let Some(delivery) = self.sim.next_delivery(tick_end) {
                if let Delivery::Datagram {
                    host,
                    from,
                    payload,
                    ..
                } = delivery
                {
                    let sender = self.devices.iter().find(|d| {
                        self.sim
                            .address(d.host, PORT)
                            .is_some_and(|a| a.ip() == from.ip())
                    });
                    if let Some(sender) = sender.map(|d| d.host) {
                        self.on_datagram(host, sender, &payload);
                    }
                }
            }
            for index in 0..self.devices.len() {
                if self.sim.rng().chance(change_per_mille) {
                    self.random_change();
                }
                if self.sim.rng().chance(400) {
                    self.start_session(index);
                }
                self.pump(index);
            }
        }
    }

    fn converged(&self) -> bool {
        let first = &self.devices[0].ledger;
        self.devices.iter().all(|device| {
            device.ledger.orphan_count() == 0
                && device.ledger.op_ids().eq(first.op_ids())
                && device.ledger.view() == first.view()
        })
    }

    /// Syncs until every replica holds the same ops, failing after `limit`.
    fn settle(&mut self, limit: Duration) {
        let end = self.sim.now() + limit;
        while !self.converged() {
            assert!(self.sim.now() < end, "replicas did not converge");
            self.run(Duration::from_secs(1), 0);
        }
    }
}

fn chaos() -> impl Strategy<Value = LinkConfig> {
    (1..60u64, 0..40u64, 0..150u16, 0..100u16, 0..200u16).prop_map(
        |(latency, jitter, loss, duplicate, reorder)| LinkConfig {
            latency: Duration::from_millis(latency),
            jitter: Duration::from_millis(jitter),
            loss_per_mille: loss,
            duplicate_per_mille: duplicate,
            reorder_per_mille: reorder,
        },
    )
}

proptest! {
    #![proptest_config(config(16))]

    /// Concurrent changes of every kind under loss, duplication, reordering and shifting
    /// partitions end in identical op sets and identical views once healed.
    #[test]
    fn replicas_converge_after_chaos(seed: u64, count in 5..=8usize, link in chaos()) {
        let mut cluster = Cluster::new(seed, count, link);
        for _ in 0..4 {
            let split = 1 + cluster.sim.rng().index(count - 1);
            let mut order: Vec<usize> = (0..count).collect();
            for i in (1..count).rev() {
                order.swap(i, cluster.sim.rng().index(i + 1));
            }
            let (left, right) = order.split_at(split);
            let (left, right) = (cluster.hosts(left), cluster.hosts(right));
            cluster.sim.partition(&[&left, &right]);
            cluster.run(Duration::from_secs(5), 150);
        }
        cluster.sim.heal();
        cluster.settle(Duration::from_secs(120));
    }

    /// An admin removes a device during a partition; joins it sponsors on its side are
    /// accepted there, but after healing it is removed everywhere and its joins are invalid.
    #[test]
    fn removal_beats_joins_sponsored_during_partition(
        seed: u64,
        joins in 1..5usize,
        link in chaos(),
    ) {
        let mut cluster = Cluster::new(seed, 5, link);
        let removed = cluster.devices[3].key.node_id();
        let invite = cluster.invites[0].verifying_key().to_bytes();
        let uses = cluster.devices[0].ledger.view().invites()[&invite].uses;
        let (minority, majority) = (cluster.hosts(&[3, 4]), cluster.hosts(&[0, 1, 2]));
        cluster.sim.partition(&[&minority, &majority]);
        cluster.act(0, OpBody::RemoveDevice { device: removed });
        let joiners: BTreeSet<NodeId> = (0..joins).map(|_| cluster.join(3, 0)).collect();
        let witness_admits = |cluster: &Cluster| {
            let view = cluster.devices[4].ledger.view();
            joiners.iter().all(|joiner| view.is_member(joiner))
        };
        for _ in 0..60 {
            if witness_admits(&cluster) {
                break;
            }
            cluster.run(Duration::from_secs(1), 0);
        }
        prop_assert!(witness_admits(&cluster), "the sponsor's side admits its joins");

        cluster.sim.heal();
        cluster.settle(Duration::from_secs(120));
        for device in &cluster.devices {
            let view = device.ledger.view();
            prop_assert!(!view.is_member(&removed));
            prop_assert!(joiners.iter().all(|joiner| !view.is_member(joiner)));
            prop_assert_eq!(view.invites()[&invite].uses, uses);
        }
    }
}
