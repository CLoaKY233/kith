//! The relay server state machine.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;

use kith_types::NodeId;

use crate::auth::verify_challenge;
use crate::frame::{Frame, NONCE_LEN, PROTOCOL_VERSION, RejectReason};

pub(crate) const MAX_QUEUED_BYTES: usize = 1 << 20;

pub(crate) const MAX_RECENT_DESTINATIONS: usize = 64;

/// Decides who may use this relay. The control plane implements it from the ledger.
pub trait Authorizer {
    /// Whether `node` is a current member of the network this relay serves.
    fn is_member(&self, node: &NodeId) -> bool;
}

impl<F: Fn(&NodeId) -> bool> Authorizer for F {
    fn is_member(&self, node: &NodeId) -> bool {
        self(node)
    }
}

/// A driver-chosen connection handle. Must be unique among open connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnId(pub u64);

/// Why the server is closing a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// Authentication failed; a [`Frame::ServerReject`] was sent first.
    Rejected(RejectReason),
    /// The same node authenticated on a newer connection.
    Replaced,
    /// The client sent a frame it may not send.
    ProtocolViolation,
    /// The node is no longer a member of the network.
    MembershipRevoked,
}

/// Something the driver must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Write `frame` to `conn`, then report its length via [`RelayServer::on_transmitted`].
    Send {
        /// The destination connection.
        conn: ConnId,
        /// The frame to write.
        frame: Frame,
    },
    /// Flush frames queued for `conn`, then close it; the server has already forgotten it.
    Close {
        /// The connection to close.
        conn: ConnId,
        /// Why.
        reason: CloseReason,
    },
}

/// [`RelayServer::on_connect`] was given a [`ConnId`] that is already open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("connection {0:?} is already open")]
pub struct DuplicateConnection(pub ConnId);

/// Whole-server counters. They saturate rather than wrap.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerStats {
    /// Packets forwarded to a destination connection.
    pub packets_forwarded: u64,
    /// Payload bytes forwarded.
    pub bytes_forwarded: u64,
    /// Frames dropped because the destination's queue was full.
    pub packets_dropped: u64,
    /// Packets addressed to a node not connected here.
    pub peer_gone: u64,
    /// Connections that failed authentication.
    pub auth_failures: u64,
    /// Connections closed for protocol violations.
    pub protocol_violations: u64,
}

/// Per-connection counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectionStats {
    /// Bytes handed to the driver and not yet reported written.
    pub queued_bytes: usize,
    /// Frames to this connection dropped because its queue was full.
    pub packets_dropped: u64,
}

#[derive(Debug)]
enum State {
    AwaitingHello { nonce: [u8; NONCE_LEN] },
    Authenticated { node: NodeId },
}

#[derive(Debug)]
struct Connection {
    state: State,
    stats: ConnectionStats,
    recent_destinations: VecDeque<NodeId>,
}

#[derive(Debug)]
struct RecentSenders(BTreeMap<NodeId, BTreeSet<ConnId>>);

impl RecentSenders {
    fn insert(&mut self, dst: NodeId, sender: ConnId) -> bool {
        self.0.entry(dst).or_default().insert(sender)
    }

    fn remove(&mut self, dst: NodeId, sender: ConnId) {
        if let Some(senders) = self.0.get_mut(&dst) {
            senders.remove(&sender);
            if senders.is_empty() {
                self.0.remove(&dst);
            }
        }
    }

    fn take(&mut self, dst: &NodeId) -> BTreeSet<ConnId> {
        self.0.remove(dst).unwrap_or_default()
    }
}

/// The relay server: authenticates members and forwards packets under their proven identity.
#[derive(Debug)]
pub struct RelayServer<A> {
    identity: NodeId,
    authorizer: A,
    connections: BTreeMap<ConnId, Connection>,
    nodes: BTreeMap<NodeId, ConnId>,
    recent_senders: RecentSenders,
    actions: VecDeque<Action>,
    stats: ServerStats,
}

impl<A: Authorizer> RelayServer<A> {
    /// A server for the relay whose node identity is `identity`.
    pub const fn new(identity: NodeId, authorizer: A) -> Self {
        Self {
            identity,
            authorizer,
            connections: BTreeMap::new(),
            nodes: BTreeMap::new(),
            recent_senders: RecentSenders(BTreeMap::new()),
            actions: VecDeque::new(),
            stats: ServerStats {
                packets_forwarded: 0,
                bytes_forwarded: 0,
                packets_dropped: 0,
                peer_gone: 0,
                auth_failures: 0,
                protocol_violations: 0,
            },
        }
    }

    /// A new stream was accepted. `nonce` must be fresh, unpredictable randomness.
    pub fn on_connect(
        &mut self,
        conn: ConnId,
        nonce: [u8; NONCE_LEN],
    ) -> Result<(), DuplicateConnection> {
        if self.connections.contains_key(&conn) {
            return Err(DuplicateConnection(conn));
        }
        self.connections.insert(
            conn,
            Connection {
                state: State::AwaitingHello { nonce },
                stats: ConnectionStats::default(),
                recent_destinations: VecDeque::new(),
            },
        );
        self.enqueue(
            conn,
            Frame::ServerChallenge {
                protocol_version: PROTOCOL_VERSION,
                nonce,
            },
        );
        Ok(())
    }

    /// The stream closed. Unknown connections are ignored.
    pub fn on_disconnect(&mut self, conn: ConnId) {
        self.forget(conn);
    }

    /// The driver wrote `bytes` of frames for `conn` to the stream.
    pub fn on_transmitted(&mut self, conn: ConnId, bytes: usize) {
        if let Some(connection) = self.connections.get_mut(&conn) {
            connection.stats.queued_bytes = connection.stats.queued_bytes.saturating_sub(bytes);
        }
    }

    /// A frame arrived on `conn`. Frames for unknown or closed connections are ignored.
    pub fn on_frame(&mut self, conn: ConnId, frame: Frame) {
        let Some(connection) = self.connections.get(&conn) else {
            return;
        };
        match connection.state {
            State::AwaitingHello { nonce } => self.on_hello(conn, nonce, &frame),
            State::Authenticated { node } => self.on_authenticated_frame(conn, node, frame),
        }
    }

    /// Closes every connection whose node is no longer a member; call after membership changes.
    pub fn refresh_membership(&mut self) {
        let revoked: Vec<ConnId> = self
            .nodes
            .iter()
            .filter(|(node, _)| !self.authorizer.is_member(node))
            .map(|(_, conn)| *conn)
            .collect();
        for conn in revoked {
            self.close(conn, CloseReason::MembershipRevoked);
        }
    }

    /// The next thing the driver must do, in order.
    pub fn poll_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    /// Whole-server counters.
    pub const fn stats(&self) -> ServerStats {
        self.stats
    }

    /// Counters for one open connection.
    pub fn connection_stats(&self, conn: ConnId) -> Option<ConnectionStats> {
        self.connections
            .get(&conn)
            .map(|connection| connection.stats)
    }

    /// The open, authenticated connection for `node`, if any.
    pub fn connection_of(&self, node: &NodeId) -> Option<ConnId> {
        self.nodes.get(node).copied()
    }

    fn on_hello(&mut self, conn: ConnId, nonce: [u8; NONCE_LEN], frame: &Frame) {
        let &Frame::ClientHello { node, signature } = frame else {
            self.reject(conn, RejectReason::PROTOCOL_VIOLATION);
            return;
        };
        if verify_challenge(node, &nonce, &self.identity, &signature).is_err() {
            self.reject(conn, RejectReason::BAD_SIGNATURE);
            return;
        }
        if !self.authorizer.is_member(&node) {
            self.reject(conn, RejectReason::NOT_MEMBER);
            return;
        }
        if let Some(connection) = self.connections.get_mut(&conn) {
            connection.state = State::Authenticated { node };
        }
        if let Some(old) = self.nodes.insert(node, conn) {
            self.close(old, CloseReason::Replaced);
        }
        self.enqueue(conn, Frame::ServerAccept);
    }

    fn on_authenticated_frame(&mut self, conn: ConnId, node: NodeId, frame: Frame) {
        match frame {
            Frame::SendPacket { dst, payload } => self.forward(conn, node, dst, payload),
            Frame::Ping { data } => {
                self.enqueue_droppable(conn, Frame::Pong { data });
            }
            Frame::Pong { .. } => {}
            Frame::ServerChallenge { .. }
            | Frame::ClientHello { .. }
            | Frame::ServerAccept
            | Frame::ServerReject { .. }
            | Frame::RecvPacket { .. }
            | Frame::PeerGone { .. }
            | Frame::Health { .. } => {
                self.stats.protocol_violations = self.stats.protocol_violations.saturating_add(1);
                self.close(conn, CloseReason::ProtocolViolation);
            }
        }
    }

    fn forward(&mut self, from: ConnId, src: NodeId, dst: NodeId, payload: Vec<u8>) {
        let Some(to) = self.nodes.get(&dst).copied() else {
            self.stats.peer_gone = self.stats.peer_gone.saturating_add(1);
            self.enqueue_droppable(from, Frame::PeerGone { node: dst });
            return;
        };
        self.note_destination(from, dst);
        let len = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        if self.enqueue_droppable(to, Frame::RecvPacket { src, payload }) {
            self.stats.packets_forwarded = self.stats.packets_forwarded.saturating_add(1);
            self.stats.bytes_forwarded = self.stats.bytes_forwarded.saturating_add(len);
        }
    }

    fn note_destination(&mut self, from: ConnId, dst: NodeId) {
        let Some(sender) = self.connections.get_mut(&from) else {
            return;
        };
        if !self.recent_senders.insert(dst, from) {
            return;
        }
        if sender.recent_destinations.len() >= MAX_RECENT_DESTINATIONS
            && let Some(evicted) = sender.recent_destinations.pop_front()
        {
            self.recent_senders.remove(evicted, from);
        }
        sender.recent_destinations.push_back(dst);
    }

    fn reject(&mut self, conn: ConnId, reason: RejectReason) {
        self.stats.auth_failures = self.stats.auth_failures.saturating_add(1);
        self.enqueue(conn, Frame::ServerReject { reason });
        self.close(conn, CloseReason::Rejected(reason));
    }

    fn close(&mut self, conn: ConnId, reason: CloseReason) {
        if self.forget(conn) {
            self.actions.push_back(Action::Close { conn, reason });
        }
    }

    fn forget(&mut self, conn: ConnId) -> bool {
        let Some(connection) = self.connections.remove(&conn) else {
            return false;
        };
        for dst in connection.recent_destinations {
            self.recent_senders.remove(dst, conn);
        }
        if let State::Authenticated { node } = connection.state
            && self.nodes.get(&node) == Some(&conn)
        {
            self.nodes.remove(&node);
            self.announce_departure(node);
        }
        true
    }

    fn announce_departure(&mut self, node: NodeId) {
        for conn in self.recent_senders.take(&node) {
            if let Some(sender) = self.connections.get_mut(&conn) {
                sender.recent_destinations.retain(|recent| *recent != node);
            }
            self.enqueue_droppable(conn, Frame::PeerGone { node });
        }
    }

    fn enqueue(&mut self, conn: ConnId, frame: Frame) {
        if let Some(connection) = self.connections.get_mut(&conn) {
            connection.stats.queued_bytes = connection
                .stats
                .queued_bytes
                .saturating_add(frame.encoded_len());
            self.actions.push_back(Action::Send { conn, frame });
        }
    }

    fn enqueue_droppable(&mut self, conn: ConnId, frame: Frame) -> bool {
        let Some(connection) = self.connections.get_mut(&conn) else {
            return false;
        };
        let queued = connection
            .stats
            .queued_bytes
            .saturating_add(frame.encoded_len());
        if queued > MAX_QUEUED_BYTES {
            connection.stats.packets_dropped = connection.stats.packets_dropped.saturating_add(1);
            self.stats.packets_dropped = self.stats.packets_dropped.saturating_add(1);
            return false;
        }
        connection.stats.queued_bytes = queued;
        self.actions.push_back(Action::Send { conn, frame });
        true
    }
}
