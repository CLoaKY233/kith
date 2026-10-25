#![expect(
    clippy::needless_pass_by_value,
    clippy::unused_self,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use std::collections::BTreeMap;

use kith_config::WgQuickConfig;
use kith_dataplane::PeerSpec;
use kith_types::{IpPrefix, NetworkId};

use crate::network::NetworkRuntime;
use crate::types::NodeError;

/// Standard WireGuard peers outside the mesh: imported exits and exported gateway clients.
#[derive(Debug, Default)]
pub(crate) struct ExternalPeers {
    imported: BTreeMap<String, Imported>,
}

/// One imported wg-quick configuration.
#[derive(Debug)]
pub(crate) struct Imported {
    /// The synthetic network id the import is addressed by.
    pub(crate) network: NetworkId,
    /// The parsed configuration.
    pub(crate) config: WgQuickConfig,
}

impl ExternalPeers {
    /// Adds an imported exit and returns the network id it is addressed by.
    pub(crate) fn import(
        &mut self,
        name: String,
        config: WgQuickConfig,
    ) -> Result<NetworkId, NodeError> {
        Err(NodeError::Pending("external: import"))
    }

    /// Renders a wg-quick file for a gateway client of `runtime`'s network.
    pub(crate) fn export(
        &mut self,
        runtime: &NetworkRuntime,
        device_name: &str,
    ) -> Result<String, NodeError> {
        Err(NodeError::Pending("external: export"))
    }

    /// An imported exit by its network id.
    pub(crate) fn get(&self, network: &NetworkId) -> Option<&Imported> {
        self.imported
            .values()
            .find(|imported| imported.network == *network)
    }

    /// Dataplane peers for every import and exported client.
    pub(crate) fn peer_specs(&self) -> Result<Vec<PeerSpec>, NodeError> {
        Err(NodeError::Pending("external: peer specs"))
    }

    /// Routes the OS must send into the tunnel for imported exits.
    pub(crate) fn routes(&self) -> Vec<IpPrefix> {
        Vec::new()
    }
}
