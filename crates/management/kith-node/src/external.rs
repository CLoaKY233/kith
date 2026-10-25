use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use kith_config::{Endpoint, WgPublicKey, WgQuickConfig};
use kith_crypto::{AgreementSecret, Context, Hash};
use kith_dataplane::{ExternalInterface, ExternalPeer};
use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NetworkId};

use crate::network::NetworkRuntime;
use crate::types::NodeError;

const GATEWAY_NAME: &str = "kith-gateway";
const GATEWAY_RANGE: (Ipv4Addr, u8) = (Ipv4Addr::new(10, 225, 0, 0), 24);
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
    /// Adds or replaces an imported exit and returns the network id it is addressed by.
    pub(crate) async fn import(
        &mut self,
        name: String,
        config: WgQuickConfig,
    ) -> Result<NetworkId, NodeError> {
        if name.is_empty() || name == GATEWAY_NAME {
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

    /// Renders a wg-quick file for a gateway client of `runtime`'s network reached at `endpoint`.
    pub(crate) fn export(
        &mut self,
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
        gateway.clients.insert(
            device_name.to_owned(),
            Client {
                public_key: secret.public_key().to_bytes(),
                address,
            },
        );
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
