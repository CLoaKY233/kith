use std::net::SocketAddr;

use kith_ledger::{AdmissionRequest, JoinProof, Ledger};
use kith_types::{NetworkId, NodeId, Timestamp};

use crate::Presence;

/// The daemon's side of the channel, called concurrently from connection tasks.
pub trait ChannelHandler: Send + Sync + 'static {
    /// The networks this node holds a ledger for.
    fn networks(&self) -> Vec<NetworkId>;

    /// Runs `f` on `network`'s ledger, or returns `None` if this node holds none.
    fn with_ledger<R>(&self, network: NetworkId, f: impl FnOnce(&mut Ledger) -> R) -> Option<R>;

    /// Whether `node` may sync and gossip `network`; by default, membership in its view.
    fn is_member(&self, network: NetworkId, node: NodeId) -> bool {
        self.with_ledger(network, |ledger| ledger.view().is_member(&node))
            .unwrap_or(false)
    }

    /// Admits `proof.device`: checks expiry, then signs and inserts the `Join` op, or says why not.
    fn handle_join(&self, network: NetworkId, proof: JoinProof) -> Result<(), String>;

    /// The member `from` gossiped endpoints; they are unverified claims.
    fn on_endpoints(
        &self,
        network: NetworkId,
        from: NodeId,
        entries: Vec<(NodeId, Vec<SocketAddr>)>,
    );

    /// The member `from` says which of the plain WireGuard members it is the gateway for
    /// are online; only claims about its own members should be believed.
    fn on_presence(&self, network: NetworkId, from: NodeId, entries: Vec<(NodeId, Presence)>);

    /// Sync or a join changed `network`'s ledger; the channel already resyncs its other peers.
    fn on_ledger_changed(&self, network: NetworkId);

    /// A device that is not a member asks, from `from`, to be added. `Ok` holds it pending
    /// for an admin; `Err` says why not, such as that an admin denied it.
    fn handle_ask(&self, _request: AdmissionRequest, _from: SocketAddr) -> Result<(), String> {
        Err(String::from("this device does not take join requests"))
    }

    /// The member `from` gossiped the join requests it holds pending and the ones admins
    /// denied; signatures are unchecked.
    fn on_admissions(
        &self,
        _network: NetworkId,
        _from: NodeId,
        _requests: Vec<AdmissionRequest>,
        _denied: Vec<(NodeId, Timestamp)>,
    ) {
    }
}
