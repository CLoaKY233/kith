#![expect(
    clippy::unused_self,
    unused_variables,
    reason = "skeleton bodies; remove once implemented"
)]

use std::path::Path;

use kith_identity::DeviceIdentity;
use kith_ledger::{Ledger, SignedOp};
use kith_store::{FsStore, MemoryStore, Store};
use kith_types::NetworkId;

use crate::types::NodeError;

/// The state directory: device identity and one persisted ledger per network.
pub(crate) struct StateDir {
    store: Box<dyn Store>,
}

impl core::fmt::Debug for StateDir {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StateDir").finish_non_exhaustive()
    }
}

impl StateDir {
    /// Opens or creates the state directory at `root`.
    pub(crate) fn open(root: &Path) -> Result<Self, NodeError> {
        Ok(Self {
            store: Box::new(FsStore::open(root)?),
        })
    }

    /// A state directory that lives only in memory.
    pub(crate) fn in_memory() -> Self {
        Self {
            store: Box::new(MemoryStore::new()),
        }
    }

    /// Loads `identity/@device`, or generates it and stores it as a secret.
    pub(crate) fn load_or_create_identity(&self) -> Result<DeviceIdentity, NodeError> {
        Err(NodeError::Pending("state: load or create identity"))
    }

    /// Loads the op list of every persisted network.
    pub(crate) fn load_networks(&self) -> Result<Vec<(NetworkId, Vec<SignedOp>)>, NodeError> {
        Err(NodeError::Pending("state: load networks"))
    }

    /// Persists every op of `ledger` under `networks/@<network-id>`.
    pub(crate) fn save_network(&self, ledger: &Ledger) -> Result<(), NodeError> {
        Err(NodeError::Pending("state: save network"))
    }

    /// Forgets a network's ledger.
    pub(crate) fn remove_network(&self, network: NetworkId) -> Result<(), NodeError> {
        Err(NodeError::Pending("state: remove network"))
    }
}
