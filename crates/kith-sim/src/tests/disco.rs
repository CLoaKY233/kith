//! Hole punching: two [`PeerPaths`] behind NATs, with call-me-maybe carried by the relay.

use std::net::SocketAddr;
use std::time::Duration;

use kith_crypto::SigningKey;
use kith_disco::{Event, Message, PathTarget, PeerPaths};
use kith_relay_proto::Event as RelayEvent;
use kith_types::NodeId;
use proptest::prelude::*;

use super::{RelayHost, RelayLink, config};
use crate::{Delivery, HostId, LinkConfig, NatConfig, Sim};

const PORT: u16 = 41_641;
const STUN_PORT: u16 = 3478;
const TICK: Duration = Duration::from_millis(100);
const STUN: u8 = b'S';
const DISCO: u8 = b'D';

struct Peer {
    host: HostId,
    remote: NodeId,
    paths: PeerPaths,
    relay: RelayLink,
    /// Every path the state machine reported, with when.
    reported: Vec<(Duration, Option<PathTarget>)>,
    /// Call-me-maybes received through the relay.
    calls_via_relay: u32,
}

struct World<A> {
    sim: Sim,
    relay: RelayHost<A>,
    stun: HostId,
    peers: [Peer; 2],
}

fn disco(message: &Message) -> Vec<u8> {
    let mut bytes = vec![DISCO];
    message.encode(&mut bytes);
    bytes
}

impl<A: kith_relay_proto::Authorizer> World<A> {
    fn peer_index(&self, host: HostId) -> Option<usize> {
        self.peers.iter().position(|peer| peer.host == host)
    }

    /// Sends what peer `index`'s path state machine has due and records its path changes.
    fn drain(&mut self, index: usize) {
        let now = self.sim.now();
        let peer = &mut self.peers[index];
        while let Some(transmit) = peer.paths.poll_transmit(now) {
            let bytes = disco(&transmit.message);
            match transmit.to {
                PathTarget::Direct(to) => self.sim.send_datagram(peer.host, PORT, to, &bytes),
                PathTarget::Relay => peer.relay.send(&mut self.sim, peer.remote, bytes),
            }
        }
        while let Some(event) = peer.paths.poll_event() {
            if let Event::PathChanged(path) = event {
                peer.reported.push((now, path));
            }
        }
    }

    fn on_datagram(&mut self, host: HostId, from: SocketAddr, to: SocketAddr, payload: &[u8]) {
        let now = self.sim.now();
        if host == self.stun {
            let observed = from.to_string().into_bytes();
            let reply = [&[STUN][..], &observed].concat();
            self.sim.send_datagram(host, STUN_PORT, from, &reply);
            return;
        }
        let Some(index) = self.peer_index(host) else {
            return;
        };
        let peer = &mut self.peers[index];
        match payload.split_first() {
            Some((&STUN, observed)) => {
                let observed: SocketAddr = std::str::from_utf8(observed).unwrap().parse().unwrap();
                peer.paths.set_local_endpoints(&[observed]);
                peer.paths.request_hole_punch(now);
            }
            Some((&DISCO, message)) => match Message::parse(message) {
                Ok(Message::Ping(ping)) => {
                    if let Some(pong) = peer.paths.handle_ping(now, from, &ping) {
                        let PathTarget::Direct(back) = pong.to else {
                            return;
                        };
                        let bytes = disco(&pong.message);
                        self.sim.send_datagram(host, to.port(), back, &bytes);
                    }
                }
                Ok(Message::Pong(pong)) => {
                    peer.paths.handle_pong(now, from, &pong);
                }
                Ok(Message::CallMeMaybe(_)) | Err(_) => {}
            },
            _ => {}
        }
        self.drain(index);
    }

    fn on_stream(&mut self, host: HostId, from: HostId, bytes: &[u8]) {
        if host == self.relay.host {
            self.relay.on_bytes(&mut self.sim, from, bytes);
            return;
        }
        let Some(index) = self.peer_index(host) else {
            return;
        };
        let now = self.sim.now();
        let peer = &mut self.peers[index];
        for event in peer.relay.on_bytes(&mut self.sim, bytes) {
            if let RelayEvent::Packet { src, payload } = event
                && src == peer.remote
                && let Some((&DISCO, message)) = payload.split_first()
                && let Ok(Message::CallMeMaybe(call)) = Message::parse(message)
            {
                peer.calls_via_relay += 1;
                peer.paths.handle_call_me_maybe(now, &call);
            }
        }
        self.drain(index);
    }

    /// Runs until `end`. Both peers have data flowing the whole time.
    fn run_until(&mut self, end: Duration) {
        while self.sim.now() < end {
            let tick_end = self.sim.now() + TICK;
            while let Some(delivery) = self.sim.next_delivery(tick_end) {
                match delivery {
                    Delivery::Datagram {
                        host,
                        from,
                        to,
                        payload,
                    } => self.on_datagram(host, from, to, &payload),
                    Delivery::Stream { host, from, bytes } => self.on_stream(host, from, &bytes),
                }
            }
            let now = self.sim.now();
            for index in 0..self.peers.len() {
                self.peers[index].paths.note_activity(now);
                self.drain(index);
            }
        }
    }

    fn both_direct(&self) -> bool {
        self.peers
            .iter()
            .all(|peer| matches!(peer.paths.best_path(), Some(PathTarget::Direct(_))))
    }
}

/// Two devices behind `nat`, both on one relay, each learning its reflexive address by STUN.
fn world(seed: u64, nat: NatConfig, link: LinkConfig) -> World<impl kith_relay_proto::Authorizer> {
    let mut sim = Sim::new(seed);
    sim.set_link(link);
    let keys = [
        SigningKey::from_seed(&[1; 32]),
        SigningKey::from_seed(&[2; 32]),
    ];
    let ids = [keys[0].node_id(), keys[1].node_id()];
    let relay_key = SigningKey::from_seed(&[9; 32]);
    let mut relay = RelayHost::new(&mut sim, &relay_key, move |node: &NodeId| {
        ids.contains(node)
    });
    let stun = sim.add_server();
    let stun_address = sim.address(stun, STUN_PORT).unwrap();
    let peers = keys.map(|key| {
        let host = sim.add_host(Some(nat));
        let local = key.node_id();
        let peer = if local == ids[0] { ids[1] } else { ids[0] };
        let paths = PeerPaths::new(local, peer, sim.rng()).unwrap();
        relay.connect(&mut sim, host);
        sim.send_datagram(host, PORT, stun_address, &[STUN]);
        Peer {
            host,
            remote: peer,
            paths,
            relay: RelayLink::new(host, relay.host, key, relay_key.node_id()),
            reported: Vec::new(),
            calls_via_relay: 0,
        }
    });
    World {
        sim,
        relay,
        stun,
        peers,
    }
}

fn link() -> impl Strategy<Value = LinkConfig> {
    (5..80u64, 0..20u64, 0..30u16).prop_map(|(latency, jitter, loss)| LinkConfig {
        latency: Duration::from_millis(latency),
        jitter: Duration::from_millis(jitter),
        loss_per_mille: loss,
        ..LinkConfig::default()
    })
}

proptest! {
    #![proptest_config(config(12))]

    /// Behind endpoint-independent NATs, the relayed call-me-maybe lets both sides punch
    /// through to a direct path within a bounded time.
    #[test]
    fn easy_nats_go_direct_via_relayed_call_me_maybe(seed: u64, link in link()) {
        let mut world = world(seed, NatConfig::EASY, link);
        let deadline = Duration::from_secs(30);
        while !world.both_direct() && world.sim.now() < deadline {
            let next = world.sim.now() + Duration::from_secs(1);
            world.run_until(next);
        }
        prop_assert!(world.both_direct(), "no direct path by {:?}", deadline);
        prop_assert!(world.peers.iter().any(|peer| peer.calls_via_relay > 0));
    }

    /// Behind symmetric NATs on both sides punching cannot work: both peers keep trying,
    /// yet stay on the relay and never report another path.
    #[test]
    fn hard_nats_stay_on_relay_without_flapping(seed: u64, link in link()) {
        let mut world = world(seed, NatConfig::HARD, link);
        world.run_until(Duration::from_secs(90));
        for peer in &world.peers {
            prop_assert!(peer.calls_via_relay > 0);
            prop_assert!(
                peer.reported.iter().all(|(_, path)| *path == Some(PathTarget::Relay)),
                "path flapped: {:?}",
                peer.reported
            );
            prop_assert_eq!(peer.paths.best_path(), Some(PathTarget::Relay));
        }
    }
}
