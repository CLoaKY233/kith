use std::net::SocketAddr;

use kith_ledger::{JoinProof, Ledger};
use kith_types::{NetworkId, NodeId};

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
}
