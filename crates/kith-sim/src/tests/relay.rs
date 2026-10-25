//! Relay safety: hostile clients against a [`RelayServer`] serving honest members.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

use kith_crypto::{Signature, SigningKey};
use kith_relay_proto::auth::sign_challenge;
use kith_relay_proto::{Authorizer, ConnId, Decoder, Event, Frame};
use kith_types::NodeId;
use proptest::prelude::*;

use super::{RelayHost, RelayLink, config, encode};
use crate::{Delivery, HostId, Sim};

const MEMBERS: usize = 3;
const ATTACKERS: usize = 2;

#[derive(Debug, Clone)]
enum Step {
    /// An honest member sends a uniquely tagged packet to another.
    Send {
        from: usize,
        to: usize,
    },
    /// An admin removes a member; the relay is told to re-check membership.
    Remove {
        member: usize,
    },
    /// An attacker opens a fresh connection.
    Reconnect {
        attacker: usize,
    },
    /// An attacker writes a hostile frame.
    Attack {
        attacker: usize,
        attack: Attack,
    },
    Wait,
}

#[derive(Debug, Clone)]
enum Attack {
    /// Claims a member's identity with a made-up signature.
    ForgedHello {
        victim: usize,
        signature: [u8; 32],
    },
    /// Claims a member's identity with the member's genuine answer to another challenge.
    ReplayedHello {
        victim: usize,
        nonce: [u8; 32],
    },
    /// Relays our challenge to a member as if from another relay, then presents its answer.
    CrossRelayHello {
        victim: usize,
    },
    /// Authenticates honestly as itself, a non-member.
    OwnHello,
    /// Asks the relay to deliver a packet to a member.
    SendPacket {
        victim: usize,
    },
    /// Sends a server-only frame claiming to carry a member's packet.
    SpoofedRecv {
        claimed: usize,
    },
    Garbage(Vec<u8>),
}

fn step() -> impl Strategy<Value = Step> {
    let member = 0..MEMBERS;
    let attacker = 0..ATTACKERS;
    let attack = prop_oneof![
        (member.clone(), any::<[u8; 32]>())
            .prop_map(|(victim, signature)| Attack::ForgedHello { victim, signature }),
        (member.clone(), any::<[u8; 32]>())
            .prop_map(|(victim, nonce)| Attack::ReplayedHello { victim, nonce }),
        member
            .clone()
            .prop_map(|victim| Attack::CrossRelayHello { victim }),
        Just(Attack::OwnHello),
        member
            .clone()
            .prop_map(|victim| Attack::SendPacket { victim }),
        member
            .clone()
            .prop_map(|claimed| Attack::SpoofedRecv { claimed }),
        prop::collection::vec(any::<u8>(), 0..40).prop_map(Attack::Garbage),
    ];
    prop_oneof![
        4 => (member.clone(), member.clone()).prop_map(|(from, to)| Step::Send { from, to }),
        1 => member.prop_map(|member| Step::Remove { member }),
        1 => attacker.clone().prop_map(|attacker| Step::Reconnect { attacker }),
        6 => (attacker, attack).prop_map(|(attacker, attack)| Step::Attack { attacker, attack }),
        2 => Just(Step::Wait),
    ]
}

struct Attacker {
    host: HostId,
    key: SigningKey,
    decoder: Decoder,
    nonce: Option<[u8; 32]>,
}

/// The relay's view of membership, which the test changes as an admin would.
#[derive(Clone)]
struct Members(Rc<RefCell<BTreeSet<NodeId>>>);

impl Authorizer for Members {
    fn is_member(&self, node: &NodeId) -> bool {
        self.0.borrow().contains(node)
    }
}

struct Harness {
    sim: Sim,
    relay_key: SigningKey,
    relay: RelayHost<Members>,
    members: Members,
    keys: Vec<SigningKey>,
    ids: Vec<NodeId>,
    honest_conns: Vec<ConnId>,
    links: Vec<RelayLink>,
    attackers: Vec<Attacker>,
    /// Every honest payload, with its true sender and when it was sent.
    sent: BTreeMap<Vec<u8>, (NodeId, Duration)>,
    removed: BTreeMap<NodeId, Duration>,
    /// Packets between members that were never removed; all must arrive.
    expected: BTreeSet<(usize, Vec<u8>)>,
    received: BTreeSet<(usize, Vec<u8>)>,
    counter: u64,
}

impl Harness {
    fn new(seed: u64) -> Self {
        let mut sim = Sim::new(seed);
        let relay_key = SigningKey::from_seed(&[200; 32]);
        let keys: Vec<SigningKey> = (1..=MEMBERS)
            .map(|i| SigningKey::from_seed(&[u8::try_from(i).unwrap(); 32]))
            .collect();
        let ids: Vec<NodeId> = keys.iter().map(SigningKey::node_id).collect();
        let members = Members(Rc::new(RefCell::new(ids.iter().copied().collect())));
        let mut relay = RelayHost::new(&mut sim, &relay_key, members.clone());
        let links: Vec<RelayLink> = keys
            .iter()
            .map(|key| {
                let host = sim.add_host(None);
                relay.connect(&mut sim, host);
                let key = SigningKey::from_seed(&key.to_seed());
                RelayLink::new(host, relay.host, key, relay_key.node_id())
            })
            .collect();
        let honest_conns = links.iter().map(|link| relay.conns[&link.host].0).collect();
        let attackers = (0..ATTACKERS)
            .map(|i| {
                let host = sim.add_host(None);
                relay.connect(&mut sim, host);
                Attacker {
                    host,
                    key: SigningKey::from_seed(&[u8::try_from(i).unwrap() + 100; 32]),
                    decoder: Decoder::new(),
                    nonce: None,
                }
            })
            .collect();
        Self {
            sim,
            relay_key,
            relay,
            members,
            keys,
            ids,
            honest_conns,
            links,
            attackers,
            sent: BTreeMap::new(),
            removed: BTreeMap::new(),
            expected: BTreeSet::new(),
            received: BTreeSet::new(),
            counter: 0,
        }
    }

    /// Delivers everything due within `duration`, checking every packet a member receives.
    fn run(&mut self, duration: Duration) -> Result<(), TestCaseError> {
        let until = self.sim.now() + duration;
        while let Some(delivery) = self.sim.next_delivery(until) {
            let Delivery::Stream { host, from, bytes } = delivery else {
                continue;
            };
            if host == self.relay.host {
                self.relay.on_bytes(&mut self.sim, from, &bytes);
            } else if let Some(index) = self.links.iter().position(|link| link.host == host) {
                for event in self.links[index].on_bytes(&mut self.sim, &bytes) {
                    if let Event::Packet { src, payload } = event {
                        let Some(&(sender, at)) = self.sent.get(&payload) else {
                            return Err(TestCaseError::fail("a forged packet was delivered"));
                        };
                        prop_assert_eq!(src, sender, "src is not the sender");
                        let gone = self.removed.get(&sender);
                        prop_assert!(gone.is_none_or(|gone| at < *gone), "sent after removal");
                        self.received.insert((index, payload));
                    }
                }
            } else if let Some(attacker) = self.attackers.iter_mut().find(|a| a.host == host) {
                attacker.decoder.push(&bytes);
                while let Ok(Some(frame)) = attacker.decoder.next_frame() {
                    let served = matches!(frame, Frame::ServerAccept | Frame::RecvPacket { .. });
                    prop_assert!(!served, "an attacker was served");
                    if let Frame::ServerChallenge { nonce, .. } = frame {
                        attacker.nonce = Some(nonce);
                    }
                }
            }
        }
        for (id, honest) in self.ids.iter().zip(&self.honest_conns) {
            let conn = self.relay.server.connection_of(id);
            prop_assert!(conn.is_none_or(|conn| conn == *honest), "identity hijacked");
        }
        for attacker in &self.attackers {
            prop_assert!(
                self.relay
                    .server
                    .connection_of(&attacker.key.node_id())
                    .is_none()
            );
        }
        Ok(())
    }

    fn apply(&mut self, step: Step) {
        match step {
            Step::Send { from, to } => {
                self.counter += 1;
                let payload = [&b"member"[..], &self.counter.to_le_bytes()].concat();
                let (sender, receiver) = (self.ids[from], self.ids[to]);
                self.sent.insert(payload.clone(), (sender, self.sim.now()));
                if !self.removed.contains_key(&sender) && !self.removed.contains_key(&receiver) {
                    self.expected.insert((to, payload.clone()));
                }
                self.links[from].send(&mut self.sim, receiver, payload);
            }
            Step::Remove { member } => {
                let id = self.ids[member];
                self.members.0.borrow_mut().remove(&id);
                self.removed.entry(id).or_insert(self.sim.now());
                self.relay.server.refresh_membership();
                self.relay.flush(&mut self.sim);
            }
            Step::Reconnect { attacker } => {
                let attacker = &mut self.attackers[attacker];
                attacker.decoder = Decoder::new();
                attacker.nonce = None;
                self.relay.connect(&mut self.sim, attacker.host);
            }
            Step::Attack { attacker, attack } => self.attack(attacker, attack),
            Step::Wait => {}
        }
    }

    fn attack(&mut self, attacker: usize, attack: Attack) {
        let attacker = &self.attackers[attacker];
        let relay = self.relay_key.node_id();
        let nonce = attacker.nonce.unwrap_or_default();
        let hello = |node, signature| Frame::ClientHello { node, signature };
        let frame = match attack {
            Attack::ForgedHello { victim, signature } => {
                let mut bytes = [0; 64];
                bytes[..32].copy_from_slice(&signature);
                hello(self.ids[victim], Signature::from_bytes(bytes))
            }
            Attack::ReplayedHello { victim, nonce } => hello(
                self.ids[victim],
                sign_challenge(&self.keys[victim], &nonce, &relay),
            ),
            Attack::CrossRelayHello { victim } => {
                let other_relay = SigningKey::from_seed(&[201; 32]).node_id();
                hello(
                    self.ids[victim],
                    sign_challenge(&self.keys[victim], &nonce, &other_relay),
                )
            }
            Attack::OwnHello => hello(
                attacker.key.node_id(),
                sign_challenge(&attacker.key, &nonce, &relay),
            ),
            Attack::SendPacket { victim } => Frame::SendPacket {
                dst: self.ids[victim],
                payload: b"attacker".to_vec(),
            },
            Attack::SpoofedRecv { claimed } => Frame::RecvPacket {
                src: self.ids[claimed],
                payload: b"spoofed".to_vec(),
            },
            Attack::Garbage(bytes) => {
                self.sim.send_stream(attacker.host, self.relay.host, bytes);
                return;
            }
        };
        self.sim
            .send_stream(attacker.host, self.relay.host, encode(&frame));
    }
}

proptest! {
    #![proptest_config(config(64))]

    /// Whatever hostile clients send, a member only ever receives packets that a member
    /// actually sent, with that member as `src`, and never after the sender was removed.
    /// Honest packets between members that stay members all arrive.
    #[test]
    fn relay_delivers_only_authenticated_member_packets(
        seed: u64,
        steps in prop::collection::vec(step(), 1..60),
    ) {
        let mut harness = Harness::new(seed);
        harness.run(Duration::from_secs(1))?;
        prop_assert!(harness.links.iter().all(|link| link.client.is_connected()));
        for step in steps {
            harness.apply(step);
            harness.run(Duration::from_millis(30))?;
        }
        harness.run(Duration::from_secs(1))?;
        prop_assert!(harness.expected.is_subset(&harness.received), "an honest packet was lost");
    }
}
