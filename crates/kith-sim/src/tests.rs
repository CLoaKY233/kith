//! System properties of the cores running together under the simulator.

mod disco;
mod ledger;
mod relay;

use std::collections::BTreeMap;

use kith_crypto::SigningKey;
use kith_relay_proto::{
    Action, Authorizer, ConnId, Decoder, Event, Frame, RelayClient, RelayServer,
};
use proptest::test_runner::{Config, RngSeed};

use crate::{HostId, Sim};

/// Few cases, a fixed proptest seed: the run is reproducible and fits the time budget.
fn config(cases: u32) -> Config {
    Config {
        cases,
        rng_seed: RngSeed::Fixed(0x6b69_7468),
        failure_persistence: None,
        ..Config::default()
    }
}

fn encode(frame: &Frame) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame.encode(&mut bytes).unwrap();
    bytes
}

/// A relay host running a [`RelayServer`] over simulated streams.
struct RelayHost<A> {
    host: HostId,
    server: RelayServer<A>,
    conns: BTreeMap<HostId, (ConnId, Decoder)>,
    next_conn: u64,
}

impl<A: Authorizer> RelayHost<A> {
    fn new(sim: &mut Sim, key: &SigningKey, authorizer: A) -> Self {
        Self {
            host: sim.add_server(),
            server: RelayServer::new(key.node_id(), authorizer),
            conns: BTreeMap::new(),
            next_conn: 0,
        }
    }

    /// Accepts a fresh stream from `client`, replacing any older one from that host.
    fn connect(&mut self, sim: &mut Sim, client: HostId) -> ConnId {
        if let Some((old, _)) = self.conns.remove(&client) {
            self.server.on_disconnect(old);
        }
        self.next_conn += 1;
        let conn = ConnId(self.next_conn);
        let mut nonce = [0; 32];
        sim.rng().fill_bytes(&mut nonce);
        self.server.on_connect(conn, nonce).unwrap();
        self.conns.insert(client, (conn, Decoder::new()));
        self.flush(sim);
        conn
    }

    fn on_bytes(&mut self, sim: &mut Sim, from: HostId, bytes: &[u8]) {
        let Some((conn, decoder)) = self.conns.get_mut(&from) else {
            return;
        };
        let conn = *conn;
        decoder.push(bytes);
        let mut frames = Vec::new();
        let mut broken = false;
        loop {
            match decoder.next_frame() {
                Ok(Some(frame)) => frames.push(frame),
                Ok(None) => break,
                Err(_) => {
                    broken = true;
                    break;
                }
            }
        }
        for frame in frames {
            self.server.on_frame(conn, frame);
        }
        if broken {
            self.conns.remove(&from);
            self.server.on_disconnect(conn);
        }
        self.flush(sim);
    }

    fn flush(&mut self, sim: &mut Sim) {
        while let Some(action) = self.server.poll_action() {
            match action {
                Action::Send { conn, frame } => {
                    let bytes = encode(&frame);
                    self.server.on_transmitted(conn, bytes.len());
                    if let Some(client) = self.client_of(conn) {
                        sim.send_stream(self.host, client, bytes);
                    }
                }
                Action::Close { conn, .. } => {
                    if let Some(client) = self.client_of(conn) {
                        self.conns.remove(&client);
                    }
                }
            }
        }
    }

    fn client_of(&self, conn: ConnId) -> Option<HostId> {
        self.conns
            .iter()
            .find(|(_, (open, _))| *open == conn)
            .map(|(host, _)| *host)
    }
}

/// A device's end of its relay stream.
struct RelayLink {
    host: HostId,
    relay: HostId,
    client: RelayClient<SigningKey>,
    decoder: Decoder,
}

impl RelayLink {
    fn new(host: HostId, relay: HostId, key: SigningKey, relay_id: kith_types::NodeId) -> Self {
        Self {
            host,
            relay,
            client: RelayClient::new(key, relay_id),
            decoder: Decoder::new(),
        }
    }

    fn on_bytes(&mut self, sim: &mut Sim, bytes: &[u8]) -> Vec<Event> {
        self.decoder.push(bytes);
        while let Ok(Some(frame)) = self.decoder.next_frame() {
            let _closed = self.client.handle_frame(frame);
        }
        self.flush(sim);
        std::iter::from_fn(|| self.client.poll_event()).collect()
    }

    fn send(&mut self, sim: &mut Sim, dst: kith_types::NodeId, payload: Vec<u8>) {
        let _dropped = self.client.send(dst, payload);
        self.flush(sim);
    }

    fn flush(&mut self, sim: &mut Sim) {
        while let Some(frame) = self.client.poll_transmit() {
            sim.send_stream(self.host, self.relay, encode(&frame));
        }
    }
}
