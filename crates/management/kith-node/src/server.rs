use core::net::SocketAddr;
use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock};

use kith_config::ServerConfig;
use kith_identity::DeviceIdentity;
use kith_relay::{Server, ServerOptions};
use kith_stun::{encode_binding_response, is_stun, parse_binding_request};
use kith_transport_quic::QuicEndpoint;
use kith_transport_ws::{TlsIdentity, WsServer};
use kith_types::NodeId;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::stealth::{QuicLinks, Tasks, accept_quic, accept_ws, serve_stun};
use crate::types::{Command, NodeError};

const DEFAULT_WS_PATH: &str = "/kith";

/// Where server mode listens, beyond the WireGuard port.
#[derive(Debug, Clone, Default)]
pub(crate) struct Listeners {
    relay: Option<SocketAddr>,
    quic: Option<SocketAddr>,
    ws: Option<SocketAddr>,
    ws_path: String,
    tls: Option<(PathBuf, PathBuf)>,
}

impl Listeners {
    /// The `[server]` listeners, with the relay on `relay` when relaying is on.
    pub(crate) fn from_config(config: &ServerConfig, relay: SocketAddr) -> Self {
        Self {
            relay: config.relay.then_some(relay),
            quic: config.quic_listen,
            ws: config.ws_listen,
            ws_path: config
                .ws_path
                .clone()
                .unwrap_or_else(|| DEFAULT_WS_PATH.to_owned()),
            tls: config.tls_cert.clone().zip(config.tls_key.clone()),
        }
    }

    /// Only a relay on `relay`, if set.
    pub(crate) fn relay_only(relay: Option<SocketAddr>) -> Self {
        Self {
            relay,
            ws_path: DEFAULT_WS_PATH.to_owned(),
            ..Self::default()
        }
    }
}

/// Server mode: relay, STUN on the relay port, QUIC and WebSocket listeners, exit NAT.
#[derive(Debug)]
pub(crate) struct ServerMode {
    relay: Option<Arc<Server>>,
    quic: Option<Arc<QuicEndpoint>>,
    members: Arc<RwLock<BTreeSet<NodeId>>>,
    exit: bool,
    _tasks: Tasks,
}

impl ServerMode {
    /// Starts every configured listener; QUIC links land in `links`, their datagrams in `commands`.
    pub(crate) async fn start(
        identity: &DeviceIdentity,
        listeners: Listeners,
        exit: bool,
        links: QuicLinks,
        commands: mpsc::Sender<Command>,
    ) -> Result<Self, NodeError> {
        let node = identity.node_id();
        let members = Arc::new(RwLock::new(BTreeSet::new()));
        let mut tasks = Tasks::default();
        let relay = match listeners.relay {
            Some(addr) => {
                let view = Arc::clone(&members);
                let authorizer = move |node: &NodeId| {
                    view.read()
                        .unwrap_or_else(PoisonError::into_inner)
                        .contains(node)
                };
                let server = Arc::new(
                    Server::bind(addr, node, authorizer, None, ServerOptions::default()).await?,
                );
                if let Some(bound) = server.local_addr() {
                    tracing::debug!(addr = %bound, "relay listening");
                    let socket = UdpSocket::bind(bound).await?;
                    tasks.0.push(tokio::spawn(serve_stun(socket)));
                }
                Some(server)
            }
            None => None,
        };
        let quic = match listeners.quic {
            Some(addr) => {
                let endpoint = Arc::new(
                    QuicEndpoint::bind(addr, &identity.signing_key().to_seed())
                        .map_err(io::Error::other)?,
                );
                tracing::debug!(addr = ?endpoint.local_addr().ok(), "QUIC listening");
                tasks.0.push(tokio::spawn(accept_quic(
                    Arc::clone(&endpoint),
                    Arc::clone(&members),
                    links,
                    commands,
                )));
                Some(endpoint)
            }
            None => None,
        };
        if let Some(addr) = listeners.ws {
            if let Some(relay) = &relay {
                let tls = listeners.tls.as_ref().map(load_tls).transpose()?;
                let server = WsServer::bind(addr, &listeners.ws_path, tls)
                    .await
                    .map_err(io::Error::other)?;
                tracing::debug!(addr = ?server.local_addr().ok(), "WebSocket listening");
                tasks
                    .0
                    .push(tokio::spawn(accept_ws(server, Arc::clone(relay))));
            } else {
                tracing::warn!("ws_listen needs the relay enabled; not listening");
            }
        }
        Ok(Self {
            relay,
            quic,
            members,
            exit,
            _tasks: tasks,
        })
    }

    /// The relay's bound address, to advertise as a `relay/<name>` record.
    pub(crate) fn relay_addr(&self) -> Option<SocketAddr> {
        self.relay.as_deref().and_then(Server::local_addr)
    }

    /// Replaces the members served, the union over every network.
    pub(crate) async fn set_members(&self, members: BTreeSet<NodeId>) -> Result<(), NodeError> {
        {
            let mut current = self.members.write().unwrap_or_else(PoisonError::into_inner);
            if *current == members {
                return Ok(());
            }
            *current = members;
        }
        if let Some(relay) = &self.relay {
            relay.refresh_membership().await;
        }
        Ok(())
    }

    /// Whether this node routes and NATs exit traffic for members.
    pub(crate) const fn is_exit(&self) -> bool {
        self.exit
    }
}

impl Drop for ServerMode {
    fn drop(&mut self) {
        if let Some(endpoint) = &self.quic {
            endpoint.close();
        }
    }
}

fn load_tls((cert, key): &(PathBuf, PathBuf)) -> Result<TlsIdentity, NodeError> {
    let invalid = |error: rustls_pki_types::pem::Error| {
        NodeError::InvalidArgument(format!("server TLS identity: {error}"))
    };
    let chain = CertificateDer::pem_file_iter(cert)
        .map_err(invalid)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid)?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(invalid)?;
    Ok(TlsIdentity { chain, key })
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
