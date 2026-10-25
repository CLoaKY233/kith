//! Client and server wired together in memory, through the real codec.

extern crate std;

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use kith_crypto::{Signature, SigningKey};
use kith_types::NodeId;

use crate::auth::sign_challenge;
use crate::frame::{Decoder, Frame, RejectReason};
use crate::server::{Action, CloseReason, ConnId, MAX_QUEUED_BYTES, RelayServer};
use crate::{Event, RelayClient, SendError};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_seed(&[seed; 32])
}

fn relay_id() -> NodeId {
    key(0xee).node_id()
}

type Members = fn(&NodeId) -> bool;

const MEMBERS: Members = members;

/// Everyone except the node with seed 99 is a member.
fn members(node: &NodeId) -> bool {
    *node != key(99).node_id()
}

struct Harness {
    server: RelayServer<Members>,
    /// Bytes written by the server per connection, decoded on delivery.
    wire: BTreeMap<ConnId, Decoder>,
    closed: Vec<(ConnId, CloseReason)>,
}

impl Harness {
    fn new() -> Self {
        Self {
            server: RelayServer::new(relay_id(), MEMBERS),
            wire: BTreeMap::new(),
            closed: Vec::new(),
        }
    }

    fn connect(&mut self, conn: u64) -> ConnId {
        let conn = ConnId(conn);
        let mut nonce = [0_u8; 32];
        nonce[..8].copy_from_slice(&conn.0.to_be_bytes());
        self.server.on_connect(conn, nonce).unwrap();
        self.wire.insert(conn, Decoder::new());
        conn
    }

    /// Runs the server's actions, encoding sent frames onto each connection's wire.
    fn drain_server(&mut self, report_written: bool) {
        while let Some(action) = self.server.poll_action() {
            match action {
                Action::Send { conn, frame } => {
                    let mut bytes = Vec::new();
                    frame.encode(&mut bytes).unwrap();
                    if report_written {
                        self.server.on_transmitted(conn, bytes.len());
                    }
                    self.wire.get_mut(&conn).unwrap().push(&bytes);
                }
                Action::Close { conn, reason } => self.closed.push((conn, reason)),
            }
        }
    }

    /// Frames the server wrote to `conn`.
    fn received(&mut self, conn: ConnId) -> Vec<Frame> {
        let decoder = self.wire.get_mut(&conn).unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            frames.push(frame);
        }
        frames
    }

    /// Delivers the client's queued frames to the server. Returns how many.
    fn client_to_server(&mut self, conn: ConnId, client: &mut RelayClient<SigningKey>) -> usize {
        let mut count = 0_usize;
        while let Some(frame) = client.poll_transmit() {
            let mut bytes = Vec::new();
            frame.encode(&mut bytes).unwrap();
            let mut decoder = Decoder::new();
            decoder.push(&bytes);
            self.server
                .on_frame(conn, decoder.next_frame().unwrap().unwrap());
            count = count.saturating_add(1);
        }
        count
    }

    /// Pumps both directions between the server and one client until quiet.
    fn pump(&mut self, conn: ConnId, client: &mut RelayClient<SigningKey>) {
        loop {
            self.drain_server(true);
            let frames = self.received(conn);
            let received = frames.len();
            for frame in frames {
                // Errors close the client; tests inspect its state afterwards.
                let _ = client.handle_frame(frame);
            }
            if received == 0 && self.client_to_server(conn, client) == 0 {
                break;
            }
        }
    }

    fn join(&mut self, conn: u64, seed: u8) -> (ConnId, RelayClient<SigningKey>) {
        let conn = self.connect(conn);
        let mut client = RelayClient::new(key(seed), relay_id());
        self.pump(conn, &mut client);
        (conn, client)
    }
}

fn events(client: &mut RelayClient<SigningKey>) -> Vec<Event> {
    core::iter::from_fn(|| client.poll_event()).collect()
}

#[test]
fn members_complete_the_handshake() {
    let mut harness = Harness::new();
    let (conn, mut client) = harness.join(1, 1);
    assert!(client.is_connected());
    assert_eq!(events(&mut client), vec![Event::Connected]);
    assert_eq!(harness.server.connection_of(&key(1).node_id()), Some(conn));
    assert_eq!(harness.closed, vec![]);
}

#[test]
fn non_members_are_rejected_and_closed() {
    let mut harness = Harness::new();
    let (conn, mut client) = harness.join(1, 99);
    assert!(!client.is_connected());
    assert_eq!(
        events(&mut client),
        vec![Event::Rejected {
            reason: RejectReason::NOT_MEMBER
        }]
    );
    assert_eq!(
        harness.closed,
        vec![(conn, CloseReason::Rejected(RejectReason::NOT_MEMBER))]
    );
    assert_eq!(harness.server.connection_of(&key(99).node_id()), None);
    assert_eq!(
        client.send(key(1).node_id(), vec![1]),
        Err(SendError::Closed)
    );
}

fn hello_rejection(hello: Frame) -> (Vec<Frame>, Vec<(ConnId, CloseReason)>) {
    let mut harness = Harness::new();
    let conn = harness.connect(1);
    harness.drain_server(true);
    harness.received(conn);
    harness.server.on_frame(conn, hello);
    harness.drain_server(true);
    (harness.received(conn), harness.closed)
}

#[test]
fn hello_needs_own_key_signature_over_this_nonce_and_relay() {
    let reject = vec![Frame::ServerReject {
        reason: RejectReason::BAD_SIGNATURE,
    }];
    let closed = vec![(
        ConnId(1),
        CloseReason::Rejected(RejectReason::BAD_SIGNATURE),
    )];

    // Garbage signature.
    let hello = Frame::ClientHello {
        node: key(1).node_id(),
        signature: Signature::from_bytes([7; 64]),
    };
    assert_eq!(hello_rejection(hello), (reject.clone(), closed.clone()));

    // A valid signature by someone else, claiming node 1.
    let mut nonce = [0_u8; 32];
    nonce[7] = 1;
    let hello = Frame::ClientHello {
        node: key(1).node_id(),
        signature: sign_challenge(&key(2), &nonce, &relay_id()),
    };
    assert_eq!(hello_rejection(hello), (reject.clone(), closed.clone()));

    // A correct answer for the same nonce but another relay: replay across relays.
    let hello = Frame::ClientHello {
        node: key(1).node_id(),
        signature: sign_challenge(&key(1), &nonce, &key(3).node_id()),
    };
    assert_eq!(hello_rejection(hello), (reject, closed));

    // Sanity: the right answer passes.
    let hello = Frame::ClientHello {
        node: key(1).node_id(),
        signature: sign_challenge(&key(1), &nonce, &relay_id()),
    };
    assert_eq!(hello_rejection(hello), (vec![Frame::ServerAccept], vec![]));
}

#[test]
fn packets_before_authentication_are_rejected() {
    let packet = Frame::SendPacket {
        dst: key(1).node_id(),
        payload: vec![1],
    };
    let (frames, closed) = hello_rejection(packet);
    assert_eq!(
        frames,
        vec![Frame::ServerReject {
            reason: RejectReason::PROTOCOL_VIOLATION
        }]
    );
    assert_eq!(
        closed,
        vec![(
            ConnId(1),
            CloseReason::Rejected(RejectReason::PROTOCOL_VIOLATION)
        )]
    );
}

#[test]
fn forwarded_packets_carry_authenticated_source_not_client_claims() {
    let mut harness = Harness::new();
    let (alice_conn, mut alice) = harness.join(1, 1);
    let (bob_conn, mut bob) = harness.join(2, 2);
    events(&mut alice);
    events(&mut bob);

    // The wire format has no source field for clients: the relay fills it in.
    alice.send(key(2).node_id(), vec![0xca, 0xfe]).unwrap();
    harness.client_to_server(alice_conn, &mut alice);
    harness.pump(bob_conn, &mut bob);
    assert_eq!(
        events(&mut bob),
        vec![Event::Packet {
            src: key(1).node_id(),
            payload: vec![0xca, 0xfe]
        }]
    );

    // A client sending a RecvPacket with a forged source is a protocol violation.
    harness.server.on_frame(
        alice_conn,
        Frame::RecvPacket {
            src: key(2).node_id(),
            payload: vec![],
        },
    );
    harness.drain_server(true);
    assert_eq!(
        harness.closed,
        vec![(alice_conn, CloseReason::ProtocolViolation)]
    );
    let stats = harness.server.stats();
    assert_eq!(stats.packets_forwarded, 1);
    assert_eq!(stats.bytes_forwarded, 2);
    assert_eq!(stats.protocol_violations, 1);
}

#[test]
fn client_holds_early_packets_and_releases_them_in_order() {
    let mut harness = Harness::new();
    let (bob_conn, mut bob) = harness.join(2, 2);
    events(&mut bob);
    let alice_conn = harness.connect(1);
    let mut alice = RelayClient::new(key(1), relay_id());
    alice.send(key(2).node_id(), vec![1]).unwrap();
    alice.send(key(2).node_id(), vec![2]).unwrap();
    harness.pump(alice_conn, &mut alice);
    harness.pump(bob_conn, &mut bob);
    let payloads: Vec<Vec<u8>> = events(&mut bob)
        .into_iter()
        .map(|event| match event {
            Event::Packet { payload, .. } => payload,
            ref event @ (Event::Connected
            | Event::Rejected { .. }
            | Event::PeerGone { .. }
            | Event::Pong { .. }
            | Event::Health { .. }) => panic!("unexpected {event:?}"),
        })
        .collect();
    assert_eq!(payloads, vec![vec![1], vec![2]]);
}

#[test]
fn newer_connection_replaces_older_for_same_node() {
    let mut harness = Harness::new();
    let (old, _) = harness.join(1, 1);
    let (new, client) = harness.join(2, 1);
    assert!(client.is_connected());
    assert_eq!(harness.closed, vec![(old, CloseReason::Replaced)]);
    assert_eq!(harness.server.connection_of(&key(1).node_id()), Some(new));
    assert_eq!(harness.server.connection_stats(old), None);

    // Late frames from the replaced connection are ignored; disconnecting it is harmless.
    harness.server.on_frame(old, Frame::Ping { data: [0; 8] });
    harness.server.on_disconnect(old);
    harness.drain_server(true);
    assert_eq!(harness.server.connection_of(&key(1).node_id()), Some(new));
}

#[test]
fn unknown_or_departed_destinations_get_peer_gone_and_ping_gets_pong() {
    let mut harness = Harness::new();
    let (conn, mut client) = harness.join(1, 1);
    events(&mut client);
    client.send(key(5).node_id(), vec![1]).unwrap();
    client.ping([4; 8]);
    harness.pump(conn, &mut client);
    assert_eq!(
        events(&mut client),
        vec![
            Event::PeerGone {
                node: key(5).node_id()
            },
            Event::Pong { data: [4; 8] }
        ]
    );
    assert_eq!(harness.server.stats().peer_gone, 1);

    // A node that disconnects becomes unknown too.
    let (bob_conn, _) = harness.join(2, 2);
    harness.server.on_disconnect(bob_conn);
    client.send(key(2).node_id(), vec![1]).unwrap();
    harness.pump(conn, &mut client);
    assert_eq!(
        events(&mut client),
        vec![Event::PeerGone {
            node: key(2).node_id()
        }]
    );
}

#[test]
fn server_drops_and_counts_beyond_queue_limit_then_recovers() {
    let mut harness = Harness::new();
    let (alice_conn, _) = harness.join(1, 1);
    let (bob_conn, _) = harness.join(2, 2);
    let payload = vec![0_u8; 1000];
    let frame_len = Frame::RecvPacket {
        src: key(1).node_id(),
        payload: payload.clone(),
    }
    .encoded_len();
    let fits = MAX_QUEUED_BYTES / frame_len;
    // Bob's driver never reports anything written, so his queue fills.
    for _ in 0..fits + 5 {
        harness.server.on_frame(
            alice_conn,
            Frame::SendPacket {
                dst: key(2).node_id(),
                payload: payload.clone(),
            },
        );
    }
    let sent_to_bob = core::iter::from_fn(|| harness.server.poll_action())
        .filter(|action| matches!(action, Action::Send { conn, .. } if *conn == bob_conn))
        .count();
    assert_eq!(sent_to_bob, fits);
    let bob = harness.server.connection_stats(bob_conn).unwrap();
    assert_eq!(bob.packets_dropped, 5);
    assert!(bob.queued_bytes <= MAX_QUEUED_BYTES);
    assert_eq!(harness.server.stats().packets_dropped, 5);

    // Once the driver reports progress, packets flow again.
    harness.server.on_transmitted(bob_conn, bob.queued_bytes);
    harness.server.on_frame(
        alice_conn,
        Frame::SendPacket {
            dst: key(2).node_id(),
            payload,
        },
    );
    assert!(matches!(
        harness.server.poll_action(),
        Some(Action::Send { conn, .. }) if conn == bob_conn
    ));
}

#[test]
fn client_bounds_pending_queue_and_payload_size() {
    let mut client = RelayClient::new(key(1), relay_id());
    let mut accepted = 0_usize;
    while client.send(key(2).node_id(), vec![0; 60_000]).is_ok() {
        accepted += 1;
    }
    assert_eq!(accepted, 4);
    assert_eq!(client.packets_dropped(), 1);
    assert_eq!(
        client.send(key(2).node_id(), vec![0; crate::frame::MAX_PAYLOAD_LEN + 1]),
        Err(SendError::TooLarge(crate::frame::MAX_PAYLOAD_LEN + 1))
    );
}

#[test]
fn revoked_members_are_disconnected_on_refresh() {
    let revoked = core::cell::Cell::new(false);
    let authorizer = |node: &NodeId| !(revoked.get() && *node == key(1).node_id());
    let mut server = RelayServer::new(relay_id(), authorizer);
    server.on_connect(ConnId(1), [0; 32]).unwrap();
    server.on_frame(
        ConnId(1),
        Frame::ClientHello {
            node: key(1).node_id(),
            signature: sign_challenge(&key(1), &[0; 32], &relay_id()),
        },
    );
    revoked.set(true);
    server.refresh_membership();
    let actions: Vec<Action> = core::iter::from_fn(|| server.poll_action()).collect();
    assert_eq!(
        actions.last(),
        Some(&Action::Close {
            conn: ConnId(1),
            reason: CloseReason::MembershipRevoked
        })
    );
    assert_eq!(server.connection_of(&key(1).node_id()), None);
}
