use std::net::SocketAddr;
use std::path::Path;

use kith_identity::DeviceIdentity;
use kith_ledger::{Ledger, SignedOp};
use kith_store::{FsStore, Key, MemoryStore, Namespace, Store};
use kith_types::{NetworkId, NodeId};

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

    /// Loads every persisted WireGuard import as `(name, wg-quick bytes)`.
    pub(crate) fn load_imports(&self) -> Result<Vec<(String, Vec<u8>)>, NodeError> {
        let namespace = Namespace::new(IMPORTS_NAMESPACE).map_err(kith_store::Error::from)?;
        let mut imports = Vec::new();
        for key in self.store.list(&namespace)? {
            let Some(name) = decode_hex(key.name.as_str()) else {
                tracing::warn!(%key, "ignoring unrecognised import entry");
                continue;
            };
            if let Some(bytes) = self.store.get(&key)? {
                imports.push((name, bytes));
            }
        }
        Ok(imports)
    }

    /// Persists an import's wg-quick text as a secret.
    pub(crate) fn save_import(&self, name: &str, conf: &[u8]) -> Result<(), NodeError> {
        let key =
            Key::parse(IMPORTS_NAMESPACE, &encode_hex(name)).map_err(kith_store::Error::from)?;
        self.store.put_secret(&key, conf)?;
        Ok(())
    }

    /// Loads the persisted gateway key and clients, if any.
    pub(crate) fn load_gateway(&self) -> Result<Option<Vec<u8>>, NodeError> {
        Ok(self.store.get(&gateway_key()?)?)
    }

    /// Persists the gateway key and clients as a secret.
    pub(crate) fn save_gateway(&self, bytes: &[u8]) -> Result<(), NodeError> {
        self.store.put_secret(&gateway_key()?, bytes)?;
        Ok(())
    }

    /// Loads the last known endpoints of every peer; a corrupt file is ignored.
    pub(crate) fn load_endpoints(&self) -> Result<Vec<(NodeId, Vec<SocketAddr>)>, NodeError> {
        let Some(bytes) = self.store.get(&endpoints_key()?)? else {
            return Ok(Vec::new());
        };
        Ok(decode_endpoints(&bytes).unwrap_or_else(|| {
            tracing::warn!("ignoring corrupt endpoint cache");
            Vec::new()
        }))
    }

    /// Persists the last known endpoints of every peer.
    pub(crate) fn save_endpoints<'a>(
        &self,
        endpoints: impl IntoIterator<Item = (&'a NodeId, &'a Vec<SocketAddr>)>,
    ) -> Result<(), NodeError> {
        let mut out = Vec::new();
        for (node, addrs) in endpoints {
            out.extend_from_slice(node.as_bytes());
            encode_varint(&mut out, addrs.len());
            for addr in addrs {
                let text = addr.to_string();
                encode_varint(&mut out, text.len());
                out.extend_from_slice(text.as_bytes());
            }
        }
        self.store.put(&endpoints_key()?, &out)?;
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
const IMPORTS_NAMESPACE: &str = "imports";
const EXTERNAL_NAMESPACE: &str = "external";
const GATEWAY_NAME: &str = "gateway";
const PEERS_NAMESPACE: &str = "peers";
const ENDPOINTS_NAME: &str = "endpoints";

fn endpoints_key() -> Result<Key, NodeError> {
    Ok(Key::parse(PEERS_NAMESPACE, ENDPOINTS_NAME).map_err(kith_store::Error::from)?)
}

fn decode_endpoints(mut input: &[u8]) -> Option<Vec<(NodeId, Vec<SocketAddr>)>> {
    let mut entries = Vec::new();
    while !input.is_empty() {
        let (id, rest) = input.split_first_chunk::<32>()?;
        input = rest;
        let count = decode_varint(&mut input)?;
        let mut addrs = Vec::new();
        for _ in 0..count {
            let len = decode_varint(&mut input)?;
            let (text, rest) = input.split_at_checked(len)?;
            input = rest;
            addrs.push(core::str::from_utf8(text).ok()?.parse().ok()?);
        }
        entries.push((NodeId::from_bytes(*id), addrs));
    }
    Some(entries)
}

fn gateway_key() -> Result<Key, NodeError> {
    Ok(Key::parse(EXTERNAL_NAMESPACE, GATEWAY_NAME).map_err(kith_store::Error::from)?)
}

fn encode_hex(text: &str) -> String {
    text.bytes().fold(String::new(), |mut out, byte| {
        for nibble in [byte >> 4, byte & 0x0f] {
            out.extend(char::from_digit(u32::from(nibble), 16));
        }
        out
    })
}

fn decode_hex(text: &str) -> Option<String> {
    let nibble = |c: u8| {
        char::from(c)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
    };
    let bytes = text
        .as_bytes()
        .chunks(2)
        .map(|pair| match pair {
            [high, low] => Some((nibble(*high)? << 4) | nibble(*low)?),
            _ => None,
        })
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

fn network_key(network: NetworkId) -> Result<Key, NodeError> {
    Ok(Key::parse(NETWORKS_NAMESPACE, &network.to_string()).map_err(kith_store::Error::from)?)
}

pub(crate) fn corrupt_value(path: &str, reason: &'static str) -> NodeError {
    NodeError::Store(kith_store::Error::Corrupt {
        path: path.into(),
        reason,
    })
}

fn corrupt(key: &Key, reason: &'static str) -> NodeError {
    NodeError::Store(kith_store::Error::Corrupt {
        path: key.to_string().into(),
        reason,
    })
}

pub(crate) fn encode_varint(out: &mut Vec<u8>, mut value: usize) {
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

pub(crate) fn decode_varint(input: &mut &[u8]) -> Option<usize> {
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
