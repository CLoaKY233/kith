use core::net::SocketAddr;
use core::time::Duration;

use kith_config::WgQuickConfig;
use kith_identity::InviteToken;
use kith_ipc::Response;
use kith_netmon::Change;
use kith_policy::Policy;
use kith_types::{NetworkId, NodeId};
use tokio::sync::oneshot;

/// Why the node failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NodeError {
    /// A ledger lock was poisoned by a panicking holder.
    #[error("a ledger lock is poisoned")]
    Poisoned,
    /// The control channel failed.
    #[error("control channel")]
    Channel(#[from] kith_channel::ChannelError),
    /// The node was not started with an in-memory TUN.
    #[error("the node has no in-memory TUN")]
    NoMemoryTun,
    /// The node's event loop has stopped.
    #[error("the node has stopped")]
    Stopped,
    /// A port kith needs is taken by another program.
    #[error(
        "port {port} ({protocol}) is already in use; another VPN (Tailscale also uses 41641) \
         or a second kith may hold it; stop it, or set network.listen_port to a free port"
    )]
    PortInUse {
        /// The port.
        port: u16,
        /// Which protocols kith needs on it.
        protocol: &'static str,
    },
    /// No joined network or peer matches.
    #[error("not found: {0}")]
    NotFound(String),
    /// A request argument was rejected.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// The configuration is invalid.
    #[error("configuration")]
    Config(#[from] kith_config::ConfigError),
    /// The state directory failed.
    #[error("state store")]
    Store(#[from] kith_store::Error),
    /// The device identity could not be loaded or created.
    #[error("identity")]
    Identity(#[from] kith_identity::IdentityError),
    /// An invite token was rejected.
    #[error("invite")]
    Invite(#[from] kith_identity::InviteError),
    /// The ledger rejected an operation.
    #[error("ledger")]
    Ledger(#[from] kith_ledger::LedgerError),
    /// A network map could not be built.
    #[error("network map")]
    Control(#[from] kith_control::ControlError),
    /// The DNS zone could not be built.
    #[error("DNS zone")]
    Zone(#[from] kith_naming::ZoneError),
    /// The dataplane failed.
    #[error("dataplane")]
    Dataplane(#[from] kith_dataplane::DataplaneError),
    /// The TUN device could not be created.
    #[error("TUN device")]
    Tun(#[from] kith_tun::TunError),
    /// The UDP socket could not be bound.
    #[error("UDP socket")]
    Udp(#[from] kith_transport_udp::UdpError),
    /// Applying or reverting OS settings failed.
    #[error("operating system")]
    Sys(#[from] kith_sys::Error),
    /// The IPC server failed.
    #[error("ipc")]
    Ipc(#[from] kith_ipc::IpcError),
    /// A relay driver failed to start.
    #[error("relay")]
    Relay(#[from] kith_relay::Error),
    /// A socket or file operation failed.
    #[error("i/o")]
    Io(#[from] std::io::Error),
}

pub(crate) type Reply<T> = oneshot::Sender<Result<T, NodeError>>;

/// A message to the event loop.
#[derive(Debug)]
pub(crate) enum Command {
    /// A local request, from IPC or a [`crate::NodeHandle`].
    Local {
        /// What to do.
        action: Action,
        /// The IPC response, or why the action failed.
        reply: Reply<Response>,
    },
    /// Something the control channel learned.
    Channel(ChannelEvent),
    /// Something the connectivity tasks learned.
    Connectivity(ConnectivityEvent),
    /// Revert the OS plan and stop the loop.
    Shutdown {
        /// Signalled once shutdown has finished.
        reply: Reply<()>,
    },
}

/// A state-changing or reading request, validated and decoded from IPC.
#[derive(Debug)]
pub(crate) enum Action {
    /// Report node status.
    Status,
    /// Found a network with this device as its only admin.
    CreateNetwork {
        /// The network name.
        name: String,
        /// This device's name in it.
        device_name: String,
    },
    /// Issue an invite for a network.
    CreateInvite {
        /// The network.
        network: NetworkId,
        /// How long the invite stays valid.
        expires_in: Duration,
        /// How many devices may redeem it.
        max_uses: u16,
    },
    /// Redeem an invite.
    Join {
        /// The parsed invite.
        token: Box<InviteToken>,
        /// This device's name in the network.
        device_name: String,
    },
    /// Leave a network and forget its ledger.
    Leave {
        /// The network.
        network: NetworkId,
    },
    /// List a network's peers.
    Peers {
        /// The network.
        network: NetworkId,
    },
    /// Write a ledger record.
    SetRecord {
        /// The network.
        network: NetworkId,
        /// The record key, such as `dns/nas`.
        key: String,
        /// The encoded record value.
        value: Vec<u8>,
    },
    /// Delete a ledger record.
    DeleteRecord {
        /// The network.
        network: NetworkId,
        /// The record key.
        key: String,
    },
    /// Replace a network's policy.
    SetPolicy {
        /// The network.
        network: NetworkId,
        /// The parsed policy.
        policy: Box<Policy>,
    },
    /// Remove a device from a network; admins only.
    RemoveDevice {
        /// The network.
        network: NetworkId,
        /// The device.
        node: NodeId,
    },
    /// Route default traffic through a peer, or stop with `None`.
    UseExit {
        /// The network.
        network: NetworkId,
        /// The exit peer.
        node: Option<NodeId>,
    },
    /// Advertise a relay as `relay/<name>`; admins only.
    AdvertiseRelay {
        /// The network.
        network: NetworkId,
        /// The relay name.
        name: String,
        /// The relay's `host:port`.
        address: String,
        /// The relay's node; `None` is this device.
        node: Option<NodeId>,
    },
    /// Remove `relay/<name>`; admins only.
    RemoveRelay {
        /// The network.
        network: NetworkId,
        /// The relay name.
        name: String,
    },
    /// Grant or revoke `exit/<node>`; admins only.
    SetExitGrant {
        /// The network.
        network: NetworkId,
        /// The node.
        node: NodeId,
        /// Grant when true.
        allowed: bool,
    },
    /// Turn server mode features on or off.
    ServerMode {
        /// Act as an exit node.
        exit: bool,
        /// Run a relay and STUN responder.
        relay: bool,
    },
    /// Add a standard WireGuard exit from a wg-quick file.
    ImportWireGuard {
        /// The local name of the import.
        name: String,
        /// The parsed configuration.
        config: Box<WgQuickConfig>,
    },
    /// Export a wg-quick file for a non-kith gateway client.
    ExportWireGuard {
        /// The network.
        network: NetworkId,
        /// The client's name.
        device_name: String,
    },
}

/// What the control channel's handler reported to the loop.
#[derive(Debug)]
pub(crate) enum ChannelEvent {
    /// The channel changed a ledger through sync or an admitted join.
    LedgerChanged {
        /// The network.
        network: NetworkId,
    },
    /// A member gossiped endpoints; they are unverified claims.
    Endpoints {
        /// The network.
        network: NetworkId,
        /// The gossiping member.
        from: NodeId,
        /// Nodes and their claimed addresses.
        entries: Vec<(NodeId, Vec<SocketAddr>)>,
    },
}

/// What a connectivity task delivered to the loop.
#[derive(Debug)]
pub(crate) enum ConnectivityEvent {
    /// A WireGuard or disco datagram arrived through a relay.
    Relayed {
        /// The sending peer.
        from: NodeId,
        /// The datagram.
        datagram: Vec<u8>,
    },
    /// The host's interfaces or gateway changed.
    NetworkChanged(Box<Change>),
    /// The router's port mapping appeared, moved or vanished.
    PortMapped(Option<SocketAddr>),
}

/// Shows an error with every cause after it, as `outer: inner: root`.
pub(crate) struct Chain<'a>(pub(crate) &'a (dyn std::error::Error + 'static));

impl core::fmt::Display for Chain<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(cause) = source {
            write!(f, ": {cause}")?;
            source = cause.source();
        }
        Ok(())
    }
}

/// Decides when changed state is worth writing: at once the first time, then at most every
/// `every`, and always when forced, such as at shutdown.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WriteThrottle {
    every: Duration,
    dirty: bool,
    written: Option<tokio::time::Instant>,
}

impl WriteThrottle {
    pub(crate) const fn new(every: Duration) -> Self {
        Self {
            every,
            dirty: false,
            written: None,
        }
    }

    /// Notes that the state changed.
    pub(crate) const fn mark(&mut self) {
        self.dirty = true;
    }

    /// Whether to write now.
    pub(crate) fn due(&self, now: tokio::time::Instant, force: bool) -> bool {
        self.dirty
            && (force
                || self
                    .written
                    .is_none_or(|at| now.saturating_duration_since(at) >= self.every))
    }

    /// Notes a successful write.
    pub(crate) const fn written(&mut self, now: tokio::time::Instant) {
        self.dirty = false;
        self.written = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::{Chain, WriteThrottle};

    #[test]
    fn writes_are_throttled_but_never_lost() {
        let every = Duration::from_secs(30);
        let start = tokio::time::Instant::now();
        let mut throttle = WriteThrottle::new(every);
        assert!(
            !throttle.due(start, true),
            "nothing changed, nothing to write"
        );

        throttle.mark();
        assert!(
            throttle.due(start, false),
            "the first change is written at once"
        );
        throttle.written(start);

        throttle.mark();
        let soon = start + Duration::from_secs(1);
        assert!(
            !throttle.due(soon, false),
            "changes within the interval wait"
        );
        assert!(
            throttle.due(soon, true),
            "shutdown writes whatever is pending"
        );
        assert!(throttle.due(start + every, false), "the interval elapsed");
    }

    #[test]
    fn chains_show_every_cause() {
        let io = std::io::Error::new(std::io::ErrorKind::AddrInUse, "Address already in use");
        let error = crate::NodeError::from(io);
        assert_eq!(Chain(&error).to_string(), "i/o: Address already in use");
    }
}
