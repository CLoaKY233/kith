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
    /// Route default traffic through a peer, or stop with `None`.
    UseExit {
        /// The network.
        network: NetworkId,
        /// The exit peer.
        node: Option<NodeId>,
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
