use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use kith_config::{Endpoint, WgPublicKey, WgQuickConfig};
use kith_crypto::{AgreementSecret, Context, Hash};
use kith_dataplane::{ExternalInterface, ExternalPeer};
use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NetworkId};

use crate::network::NetworkRuntime;
use crate::state::{StateDir, corrupt_value, decode_varint, encode_varint};
use crate::types::NodeError;

const GATEWAY_NAME: &str = "kith-gateway";
const GATEWAY_RANGE: (Ipv4Addr, u8) = (Ipv4Addr::new(10, 225, 0, 0), 24);
const MAX_IMPORT_NAME: usize = 64;
const IMPORT_CONTEXT: Context = Context::new("kith-node external import network id v1");

/// Standard WireGuard peers outside the mesh: imported exits and exported gateway clients.
#[derive(Debug, Default)]
pub(crate) struct ExternalPeers {
    imported: BTreeMap<String, ExternalInterface>,
    gateway: Option<Gateway>,
}

#[derive(Debug)]
struct Gateway {
    secret: AgreementSecret,
    range: Ipv4Prefix,
    clients: BTreeMap<String, Client>,
}

#[derive(Debug, Clone, Copy)]
struct Client {
    public_key: [u8; 32],
    address: Ipv4Addr,
}

impl ExternalPeers {
    /// Restores the persisted gateway and imports from `state`.
    pub(crate) async fn restore(state: &StateDir) -> Result<Self, NodeError> {
        let mut peers = Self {
            imported: BTreeMap::new(),
            gateway: state
                .load_gateway()?
                .map(|b| Gateway::decode(&b))
                .transpose()?,
        };
        for (name, bytes) in state.load_imports()? {
            let config = core::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| WgQuickConfig::parse(text).ok());
            let Some(config) = config else {
                tracing::warn!(%name, "ignoring unreadable persisted import");
                continue;
            };
            if let Err(error) = peers.insert_import(name.clone(), config).await {
                tracing::warn!(%name, %error, "ignoring persisted import");
            }
        }
        Ok(peers)
    }

    /// Adds or replaces an imported exit, persists it, and returns the network id it is addressed by.
    pub(crate) async fn import(
        &mut self,
        state: &StateDir,
        name: String,
        config: WgQuickConfig,
    ) -> Result<NetworkId, NodeError> {
        let network = self.insert_import(name.clone(), config).await?;
        if let Some(interface) = self.imported.get(&name) {
            state.save_import(&name, render_import(interface)?.as_bytes())?;
        }
        Ok(network)
    }

    async fn insert_import(
        &mut self,
        name: String,
        config: WgQuickConfig,
    ) -> Result<NetworkId, NodeError> {
        if name.is_empty() || name.len() > MAX_IMPORT_NAME || name == GATEWAY_NAME {
            return Err(NodeError::InvalidArgument(format!(
                "invalid import name {name:?}"
            )));
        }
        let mut peers = Vec::with_capacity(config.peers.len());
        for peer in &config.peers {
            let endpoint = match &peer.endpoint {
                None => None,
                Some(endpoint) => Some(resolve(endpoint).await?),
            };
            peers.push(ExternalPeer {
                public_key: peer.public_key.0,
                preshared_key: peer.preshared_key.as_ref().map(|key| *key.as_bytes()),
                endpoint,
                allowed_ips: peer.allowed_ips.clone(),
                persistent_keepalive: peer.persistent_keepalive,
            });
        }
        let addresses = config
            .interface
            .addresses
            .iter()
            .map(|address| host_prefix(address.addr))
            .collect::<Result<Vec<_>, _>>()?;
        let interface = ExternalInterface {
            name: name.clone(),
            private_key: *config.interface.private_key.as_bytes(),
            addresses,
            peers,
        };
        let network = NetworkId::from_bytes(Hash::of(IMPORT_CONTEXT, name.as_bytes()).to_bytes());
        self.imported.insert(name, interface);
        Ok(network)
    }

    /// Renders a wg-quick file for a gateway client of `runtime`'s network reached at `endpoint`, persisting the client.
    pub(crate) fn export(
        &mut self,
        state: &StateDir,
        runtime: &NetworkRuntime,
        device_name: &str,
        endpoint: &str,
    ) -> Result<String, NodeError> {
        if device_name.is_empty() {
            return Err(NodeError::InvalidArgument("empty device name".into()));
        }
        if endpoint.contains(['\n', '\r']) {
            return Err(NodeError::InvalidArgument("invalid endpoint".into()));
        }
        let gateway = match &mut self.gateway {
            Some(gateway) => gateway,
            None => self.gateway.insert(Gateway::new()?),
        };
        let secret = AgreementSecret::generate().map_err(random_failed)?;
        let address = gateway.allocate(device_name)?;
        let previous = gateway.clients.insert(
            device_name.to_owned(),
            Client {
                public_key: secret.public_key().to_bytes(),
                address,
            },
        );
        if let Err(error) = state.save_gateway(&gateway.encode()) {
            match previous {
                Some(client) => gateway.clients.insert(device_name.to_owned(), client),
                None => gateway.clients.remove(device_name),
            };
            return Err(error);
        }
        let map = runtime.netmap();
        let dns = map
            .resolver()
            .into_iter()
            .filter(IpAddr::is_ipv4)
            .map(|ip| ip.to_string())
            .collect::<Vec<_>>();
        let mut out = String::new();
        let private = WgPublicKey(*secret.to_bytes());
        let server = WgPublicKey(gateway.secret.public_key().to_bytes());
        let render = (|| {
            writeln!(out, "[Interface]")?;
            writeln!(out, "PrivateKey = {private}")?;
            writeln!(out, "Address = {address}/32")?;
            if !dns.is_empty() {
                writeln!(out, "DNS = {}", dns.join(", "))?;
            }
            writeln!(out)?;
            writeln!(out, "[Peer]")?;
            writeln!(out, "PublicKey = {server}")?;
            writeln!(out, "Endpoint = {endpoint}")?;
            writeln!(
                out,
                "AllowedIPs = {}, {}",
                IpPrefix::V4(map.ipv4_range()),
                IpPrefix::V4(gateway.range)
            )?;
            writeln!(out, "PersistentKeepalive = 25")
        })();
        render.map_err(|e| NodeError::InvalidArgument(e.to_string()))?;
        Ok(out)
    }

    /// Dataplane external interfaces for every import and the gateway's exported clients.
    pub(crate) fn interfaces(&self) -> Result<Vec<ExternalInterface>, NodeError> {
        let mut interfaces = self.imported.values().cloned().collect::<Vec<_>>();
        if let Some(gateway) = &self.gateway {
            interfaces.push(gateway.interface()?);
        }
        Ok(interfaces)
    }

    /// Routes the OS must send into the tunnel for imported exits.
    pub(crate) fn routes(&self) -> Vec<IpPrefix> {
        let mut routes = self
            .imported
            .values()
            .flat_map(|interface| &interface.peers)
            .flat_map(|peer| peer.allowed_ips.iter().copied())
            .collect::<Vec<_>>();
        routes.sort_unstable();
        routes.dedup();
        routes
    }
}

impl Gateway {
    fn new() -> Result<Self, NodeError> {
        Ok(Self {
            secret: AgreementSecret::generate().map_err(random_failed)?,
            range: Ipv4Prefix::new(GATEWAY_RANGE.0, GATEWAY_RANGE.1)
                .map_err(|e| NodeError::InvalidArgument(e.to_string()))?,
            clients: BTreeMap::new(),
        })
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = self.secret.to_bytes().to_vec();
        for (name, client) in &self.clients {
            encode_varint(&mut out, name.len());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&client.public_key);
            out.extend_from_slice(&client.address.octets());
        }
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, NodeError> {
        let invalid = || corrupt_value("external/@gateway", "invalid gateway state");
        let (secret, mut rest) = bytes.split_first_chunk::<32>().ok_or_else(invalid)?;
        let mut gateway = Self::new()?;
        gateway.secret = AgreementSecret::from_bytes(*secret);
        while !rest.is_empty() {
            let len = decode_varint(&mut rest).ok_or_else(invalid)?;
            let (name, tail) = rest.split_at_checked(len).ok_or_else(invalid)?;
            let (public_key, tail) = tail.split_first_chunk::<32>().ok_or_else(invalid)?;
            let (address, tail) = tail.split_first_chunk::<4>().ok_or_else(invalid)?;
            let name = core::str::from_utf8(name).map_err(|_| invalid())?;
            gateway.clients.insert(
                name.to_owned(),
                Client {
                    public_key: *public_key,
                    address: Ipv4Addr::from(*address),
                },
            );
            rest = tail;
        }
        Ok(gateway)
    }

    fn own_address(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.range.addr()) | 1)
    }

    fn allocate(&self, device_name: &str) -> Result<Ipv4Addr, NodeError> {
        if let Some(client) = self.clients.get(device_name) {
            return Ok(client.address);
        }
        let base = u32::from(self.range.addr());
        let size = 1_u32
            .checked_shl(u32::from(Ipv4Prefix::WIDTH - self.range.prefix_len()))
            .unwrap_or(0);
        (2..size.saturating_sub(1))
            .map(|offset| Ipv4Addr::from(base | offset))
            .find(|candidate| self.clients.values().all(|c| c.address != *candidate))
            .ok_or_else(|| NodeError::InvalidArgument("gateway address range is full".into()))
    }

    fn interface(&self) -> Result<ExternalInterface, NodeError> {
        let peers = self
            .clients
            .values()
            .map(|client| {
                Ok(ExternalPeer {
                    public_key: client.public_key,
                    preshared_key: None,
                    endpoint: None,
                    allowed_ips: vec![host_prefix(IpAddr::V4(client.address))?],
                    persistent_keepalive: None,
                })
            })
            .collect::<Result<Vec<_>, NodeError>>()?;
        Ok(ExternalInterface {
            name: GATEWAY_NAME.to_owned(),
            private_key: *self.secret.to_bytes(),
            addresses: vec![host_prefix(IpAddr::V4(self.own_address()))?],
            peers,
        })
    }
}

fn render_import(interface: &ExternalInterface) -> Result<String, NodeError> {
    let mut out = String::new();
    let render = (|| {
        writeln!(out, "[Interface]")?;
        writeln!(out, "PrivateKey = {}", WgPublicKey(interface.private_key))?;
        for address in &interface.addresses {
            writeln!(out, "Address = {address}")?;
        }
        for peer in &interface.peers {
            writeln!(out, "\n[Peer]")?;
            writeln!(out, "PublicKey = {}", WgPublicKey(peer.public_key))?;
            if let Some(key) = peer.preshared_key {
                writeln!(out, "PresharedKey = {}", WgPublicKey(key))?;
            }
            if let Some(endpoint) = peer.endpoint {
                writeln!(out, "Endpoint = {endpoint}")?;
            }
            for prefix in &peer.allowed_ips {
                writeln!(out, "AllowedIPs = {prefix}")?;
            }
            if let Some(keepalive) = peer.persistent_keepalive {
                writeln!(out, "PersistentKeepalive = {keepalive}")?;
            }
        }
        Ok::<(), core::fmt::Error>(())
    })();
    render.map_err(|e| NodeError::InvalidArgument(e.to_string()))?;
    Ok(out)
}

fn host_prefix(addr: IpAddr) -> Result<IpPrefix, NodeError> {
    let prefix = match addr {
        IpAddr::V4(v4) => Ipv4Prefix::new(v4, Ipv4Prefix::WIDTH).map(IpPrefix::V4),
        IpAddr::V6(v6) => Ipv6Prefix::new(v6, Ipv6Prefix::WIDTH).map(IpPrefix::V6),
    };
    prefix.map_err(|e| NodeError::InvalidArgument(e.to_string()))
}

async fn resolve(endpoint: &Endpoint) -> Result<SocketAddr, NodeError> {
    match endpoint {
        Endpoint::Addr(addr) => Ok(*addr),
        Endpoint::Host { host, port } => tokio::net::lookup_host((host.as_str(), *port))
            .await?
            .next()
            .ok_or_else(|| NodeError::NotFound(format!("endpoint host {host}"))),
    }
}

fn random_failed(error: kith_crypto::RandomError) -> NodeError {
    NodeError::Io(std::io::Error::other(error))
}
