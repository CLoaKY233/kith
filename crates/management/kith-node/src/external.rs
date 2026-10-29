use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};

use kith_config::{Endpoint, WgPublicKey, WgQuickConfig};
use kith_crypto::{AgreementSecret, Context, Hash};
use kith_dataplane::{ExternalInterface, ExternalPeer};
use kith_policy::Addresses;
use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix, NetworkId};

use crate::state::{StateDir, corrupt_value, decode_varint};
use crate::types::NodeError;

const GATEWAY_NAME: &str = "kith-gateway";
const MAX_IMPORT_NAME: usize = 64;
const IMPORT_CONTEXT: Context = Context::new("kith-node external import network id v1");

/// Standard WireGuard peers: imported exits, and the key plain WireGuard members, such as
/// phones, connect to when this device is their gateway.
#[derive(Debug, Default)]
pub(crate) struct ExternalPeers {
    imported: BTreeMap<String, ExternalInterface>,
    gateway: Option<AgreementSecret>,
}

/// What a plain WireGuard member's config holds.
pub(crate) struct ClientConfig<'a> {
    /// The member's private key. It is only ever written into the config.
    pub(crate) secret: &'a AgreementSecret,
    /// Its fixed mesh addresses.
    pub(crate) addresses: Addresses,
    /// The mesh DNS resolver.
    pub(crate) dns: IpAddr,
    /// The gateway's WireGuard public key.
    pub(crate) gateway: [u8; 32],
    /// Where the gateway is reached, as `host:port`.
    pub(crate) endpoint: &'a str,
    /// What the member sends through the gateway.
    pub(crate) allowed_ips: &'a [IpPrefix],
}

impl ExternalPeers {
    /// Restores the persisted gateway and imports from `state`.
    pub(crate) async fn restore(state: &StateDir) -> Result<Self, NodeError> {
        let mut peers = Self {
            imported: BTreeMap::new(),
            gateway: state
                .load_gateway()?
                .map(|bytes| load_gateway_key(state, &bytes))
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
                member: None,
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

    /// The gateway's public key, creating and saving the key on first use.
    pub(crate) fn gateway_public_key(&mut self, state: &StateDir) -> Result<[u8; 32], NodeError> {
        if let Some(secret) = &self.gateway {
            return Ok(secret.public_key().to_bytes());
        }
        let secret = AgreementSecret::generate().map_err(random_failed)?;
        state.save_gateway(secret.to_bytes().as_slice())?;
        let public = secret.public_key().to_bytes();
        self.gateway = Some(secret);
        Ok(public)
    }

    /// Dataplane external interfaces for every import, and for the gateway when it has
    /// `clients`.
    pub(crate) fn interfaces(&self, clients: Vec<ExternalPeer>) -> Vec<ExternalInterface> {
        let mut interfaces = self.imported.values().cloned().collect::<Vec<_>>();
        if let Some(secret) = self.gateway.as_ref().filter(|_| !clients.is_empty()) {
            interfaces.push(ExternalInterface {
                name: GATEWAY_NAME.to_owned(),
                private_key: *secret.to_bytes(),
                addresses: Vec::new(),
                peers: clients,
            });
        }
        interfaces
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

/// The saved gateway key. An older kith saved its exported devices after the key, outside
/// the ledger; they cannot be kept, since devices now need fixed addresses every member
/// agrees on, so they are named in the log and dropped from the file.
fn load_gateway_key(state: &StateDir, bytes: &[u8]) -> Result<AgreementSecret, NodeError> {
    let Some((secret, rest)) = bytes.split_first_chunk::<32>() else {
        return Err(corrupt_value("external/@gateway", "invalid gateway state"));
    };
    if !rest.is_empty() {
        let names = old_device_names(rest);
        tracing::warn!(
            devices = %names.join(", "),
            "devices added by an older kith with `kith wg export` no longer connect; add each again with `kith wg export`"
        );
        state.save_gateway(secret)?;
    }
    Ok(AgreementSecret::from_bytes(*secret))
}

/// The names in an older kith's list of exported devices: each a length, the name, a
/// 32-byte key and a 4-byte address.
fn old_device_names(mut rest: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    while let Some(len) = decode_varint(&mut rest) {
        let Some((name, tail)) = rest.split_at_checked(len) else {
            break;
        };
        names.push(String::from_utf8_lossy(name).into_owned());
        rest = tail.get(36..).unwrap_or_default();
    }
    names
}

/// A wg-quick file for a plain WireGuard member.
pub(crate) fn render_client(config: &ClientConfig<'_>) -> Result<String, NodeError> {
    let mut out = String::new();
    let render = (|| {
        writeln!(out, "[Interface]")?;
        writeln!(
            out,
            "PrivateKey = {}",
            WgPublicKey(*config.secret.to_bytes())
        )?;
        writeln!(
            out,
            "Address = {}/32, {}/128",
            config.addresses.v4, config.addresses.v6
        )?;
        writeln!(out, "DNS = {}", config.dns)?;
        writeln!(out)?;
        writeln!(out, "[Peer]")?;
        writeln!(out, "PublicKey = {}", WgPublicKey(config.gateway))?;
        writeln!(out, "Endpoint = {}", config.endpoint)?;
        let allowed: Vec<String> = config.allowed_ips.iter().map(ToString::to_string).collect();
        writeln!(out, "AllowedIPs = {}", allowed.join(", "))?;
        writeln!(out, "PersistentKeepalive = 25")
    })();
    render.map_err(|e| NodeError::InvalidArgument(e.to_string()))?;
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::{load_gateway_key, old_device_names};
    use crate::state::{StateDir, encode_varint};

    #[test]
    fn an_older_gateway_file_keeps_its_key_and_names_the_devices_it_drops() {
        let state = StateDir::in_memory();
        let mut old = vec![7; 32];
        for name in ["phone", "tablet"] {
            encode_varint(&mut old, name.len());
            old.extend_from_slice(name.as_bytes());
            old.extend_from_slice(&[1; 36]);
        }
        assert_eq!(
            old_device_names(old.get(32..).unwrap()),
            ["phone", "tablet"]
        );
        let key = load_gateway_key(&state, &old).unwrap();
        assert_eq!(*key.to_bytes(), [7; 32]);
        assert_eq!(
            state.load_gateway().unwrap().unwrap(),
            vec![7; 32],
            "the old list is gone"
        );
        assert!(
            load_gateway_key(&state, &[7; 31]).is_err(),
            "a short file is corrupt"
        );
    }
}
