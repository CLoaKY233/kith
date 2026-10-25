#![expect(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use core::net::SocketAddr;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use kith_relay::Server;
use kith_stun::{encode_binding_response, is_stun, parse_binding_request};
use kith_types::{IpPrefix, NodeId};

use crate::types::NodeError;

/// Server mode: a relay for members, a STUN responder on the WireGuard port, exit NAT.
#[derive(Debug)]
pub(crate) struct ServerMode {
    relay: Option<Server>,
    members: Arc<RwLock<BTreeSet<NodeId>>>,
    exit: bool,
}

impl ServerMode {
    /// Starts the relay on `relay_listen` when `relay` is set; `exit` enables exit NAT.
    pub(crate) async fn start(
        identity: NodeId,
        relay_listen: Option<SocketAddr>,
        exit: bool,
    ) -> Result<Self, NodeError> {
        Err(NodeError::Pending("server: start"))
    }

    /// The relay's bound address, to advertise as a `relay/<name>` record.
    pub(crate) fn relay_addr(&self) -> Option<SocketAddr> {
        self.relay.as_ref().and_then(Server::local_addr)
    }

    /// Replaces the members the relay serves, the union over every network.
    pub(crate) async fn set_members(&self, members: BTreeSet<NodeId>) -> Result<(), NodeError> {
        Err(NodeError::Pending("server: set members"))
    }

    /// Whether this node routes and NATs exit traffic for members.
    pub(crate) const fn is_exit(&self) -> bool {
        self.exit
    }

    /// Mesh sources to NAT when acting as an exit.
    pub(crate) fn nat_sources(&self, mesh: &[IpPrefix]) -> Vec<IpPrefix> {
        if self.exit { mesh.to_vec() } else { Vec::new() }
    }
}

/// Answers a STUN binding request with the observed source, or `None` if it is not one.
pub(crate) fn stun_reply(source: SocketAddr, datagram: &[u8]) -> Option<Vec<u8>> {
    if !is_stun(datagram) {
        return None;
    }
    let txid = parse_binding_request(datagram).ok()?;
    let mut out = [0; kith_stun::MAX_RESPONSE_LEN];
    Some(encode_binding_response(txid, source, &mut out).to_vec())
}
