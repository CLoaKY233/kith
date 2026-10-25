use std::path::Path;

use kith_identity::DeviceIdentity;
use kith_ledger::{Ledger, SignedOp};
use kith_store::{FsStore, Key, MemoryStore, Namespace, Store};
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
        let key = Key::parse(IDENTITY_NAMESPACE, DEVICE_NAME).map_err(kith_store::Error::from)?;
        if let Some(bytes) = self.store.get(&key)? {
            return Ok(DeviceIdentity::from_secret_bytes(&bytes)?);
        }
        let identity = DeviceIdentity::generate()?;
        self.store.put_secret(&key, &identity.to_secret_bytes())?;
        Ok(identity)
    }

    /// Loads the op list of every persisted network.
    pub(crate) fn load_networks(&self) -> Result<Vec<(NetworkId, Vec<SignedOp>)>, NodeError> {
        let namespace = Namespace::new(NETWORKS_NAMESPACE).map_err(kith_store::Error::from)?;
        let mut networks = Vec::new();
        for key in self.store.list(&namespace)? {
            let Ok(network) = key.name.as_str().parse::<NetworkId>() else {
                tracing::warn!(%key, "ignoring unrecognised network entry");
                continue;
            };
            let Some(bytes) = self.store.get(&key)? else {
                continue;
            };
            let ops = decode_ops(&bytes)
                .ok_or_else(|| corrupt(&key, "truncated op list"))?
                .into_iter()
                .map(SignedOp::from_bytes)
                .collect::<Result<Vec<_>, _>>()?;
            networks.push((network, ops));
        }
        Ok(networks)
    }

    /// Persists every op of `ledger` under `networks/@<network-id>`, genesis first.
    pub(crate) fn save_network(&self, ledger: &Ledger) -> Result<(), NodeError> {
        let genesis = ledger.genesis_id();
        let ops = ledger
            .get(&genesis)
            .into_iter()
            .chain(
                ledger
                    .op_ids()
                    .filter(|id| **id != genesis)
                    .filter_map(|id| ledger.get(id)),
            )
            .map(SignedOp::to_bytes);
        let mut out = Vec::new();
        encode_varint(&mut out, ledger.len());
        for op in ops {
            encode_varint(&mut out, op.len());
            out.extend_from_slice(&op);
        }
        self.store.put(&network_key(ledger.network_id())?, &out)?;
        Ok(())
    }

    /// Forgets a network's ledger.
    pub(crate) fn remove_network(&self, network: NetworkId) -> Result<(), NodeError> {
        self.store.delete(&network_key(network)?)?;
        Ok(())
    }
}

const IDENTITY_NAMESPACE: &str = "identity";
const DEVICE_NAME: &str = "device";
const NETWORKS_NAMESPACE: &str = "networks";

fn network_key(network: NetworkId) -> Result<Key, NodeError> {
    Ok(Key::parse(NETWORKS_NAMESPACE, &network.to_string()).map_err(kith_store::Error::from)?)
}

fn corrupt(key: &Key, reason: &'static str) -> NodeError {
    NodeError::Store(kith_store::Error::Corrupt {
        path: key.to_string().into(),
        reason,
    })
}

fn encode_varint(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let [low, ..] = value.to_le_bytes();
        value >>= 7;
        if value == 0 {
            out.push(low & 0x7f);
            return;
        }
        out.push(low | 0x80);
    }
}

fn decode_varint(input: &mut &[u8]) -> Option<usize> {
    let mut value = 0usize;
    for shift in (0..usize::BITS).step_by(7) {
        let (&byte, rest) = input.split_first()?;
        *input = rest;
        value |= usize::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn decode_ops(mut input: &[u8]) -> Option<Vec<&[u8]>> {
    let count = decode_varint(&mut input)?;
    let mut ops = Vec::with_capacity(count.min(input.len()));
    for _ in 0..count {
        let len = decode_varint(&mut input)?;
        let (op, rest) = input.split_at_checked(len)?;
        ops.push(op);
        input = rest;
    }
    input.is_empty().then_some(ops)
}
