#![expect(
    clippy::needless_pass_by_value,
    clippy::unused_self,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use core::net::SocketAddr;
use std::collections::BTreeMap;
use std::sync::Arc;

use kith_control::records::Relay;
use kith_dataplane::Endpoint;
use kith_disco::PeerPaths;
use kith_identity::DeviceIdentity;
use kith_ipc::PathKind;
use kith_netmon::Change;
use kith_types::NodeId;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::types::{Command, NodeError};

/// Something the loop must do on behalf of connectivity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Output {
    /// Send a STUN or disco datagram from the WireGuard socket.
    SendRaw {
        /// The destination.
        destination: SocketAddr,
        /// The datagram.
        datagram: Vec<u8>,
    },
    /// Feed a relayed WireGuard datagram into the dataplane.
    AcceptRelayed {
        /// The sending peer.
        peer: NodeId,
        /// The datagram.
        datagram: Vec<u8>,
    },
    /// Point a peer's WireGuard endpoint at a new path.
    PathChanged {
        /// The peer.
        peer: NodeId,
        /// The new endpoint.
        endpoint: Endpoint,
    },
}

/// Per-peer path selection, STUN, relay clients, port mapping and network changes.
#[derive(Debug)]
pub(crate) struct Connectivity {
    identity: Arc<DeviceIdentity>,
    commands: mpsc::Sender<Command>,
    peers: BTreeMap<NodeId, PeerPaths>,
    outputs: Vec<Output>,
}

impl Connectivity {
    /// Starts the netmon and port-mapping tasks, which report through `commands`.
    pub(crate) fn start(
        identity: Arc<DeviceIdentity>,
        local_port: u16,
        tun_name: Option<String>,
        commands: mpsc::Sender<Command>,
    ) -> Result<Self, NodeError> {
        Err(NodeError::Pending("connectivity: start"))
    }

    /// Replaces the set of mesh peers; new ones start probing, removed ones are dropped.
    pub(crate) fn set_peers(&mut self, peers: &[NodeId], now: Instant) {}

    /// Adds candidate addresses for a peer from gossip or its control connection.
    pub(crate) fn add_candidates(&mut self, peer: NodeId, addrs: &[SocketAddr], now: Instant) {}

    /// Connects to every advertised relay and drops clients for withdrawn ones.
    pub(crate) fn set_relays(&mut self, relays: &[Relay]) -> Result<(), NodeError> {
        Err(NodeError::Pending("connectivity: set relays"))
    }

    /// Handles a STUN or disco datagram from the dataplane's control event.
    pub(crate) fn handle_datagram(
        &mut self,
        source: SocketAddr,
        datagram: &[u8],
        now: Instant,
    ) -> Result<(), NodeError> {
        Err(NodeError::Pending("connectivity: handle datagram"))
    }

    /// Sends a WireGuard datagram for `peer` through a connected relay.
    pub(crate) fn send_relayed(
        &mut self,
        peer: NodeId,
        datagram: Vec<u8>,
    ) -> Result<(), NodeError> {
        Err(NodeError::Pending("connectivity: send relayed"))
    }

    /// Handles a datagram a relay delivered, disco or WireGuard.
    pub(crate) fn handle_relayed(
        &mut self,
        from: NodeId,
        datagram: Vec<u8>,
        now: Instant,
    ) -> Result<(), NodeError> {
        Err(NodeError::Pending("connectivity: handle relayed"))
    }

    /// Re-gathers local endpoints and re-probes every peer after a host network change.
    pub(crate) fn handle_network_change(&mut self, change: &Change, now: Instant) {}

    /// Records the router's current port mapping as a local candidate.
    pub(crate) fn set_port_mapping(&mut self, mapped: Option<SocketAddr>) {}

    /// Notes that a peer's WireGuard session is up.
    pub(crate) fn note_handshake(&mut self, peer: NodeId, now: Instant) {}

    /// Advances every timer that is due.
    pub(crate) fn handle_timeout(&mut self, now: Instant) {}

    /// The next instant [`Connectivity::handle_timeout`] must run.
    pub(crate) fn poll_timeout(&self) -> Option<Instant> {
        None
    }

    /// Takes the queued outputs.
    pub(crate) fn drain_outputs(&mut self) -> Vec<Output> {
        core::mem::take(&mut self.outputs)
    }

    /// Our endpoints worth gossiping: local, STUN-observed and port-mapped.
    pub(crate) fn local_endpoints(&self) -> Vec<SocketAddr> {
        Vec::new()
    }

    /// The path currently used for a peer.
    pub(crate) fn path_kind(&self, peer: &NodeId) -> Option<PathKind> {
        None
    }
}
