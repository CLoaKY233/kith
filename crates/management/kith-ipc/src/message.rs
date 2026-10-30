use std::collections::BTreeMap;

use core::net::{Ipv4Addr, Ipv6Addr};

use kith_types::{NetworkId, NodeId};
use serde::{Deserialize, Serialize};

/// A command from the CLI to the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Request {
    /// Opens the conversation and negotiates the protocol version.
    Hello {
        /// The client's [`crate::PROTOCOL_VERSION`].
        protocol: u32,
        /// The client's software version, for diagnostics.
        client_version: String,
    },
    /// Reports the daemon's state. Allowed for any local user.
    Status,
    /// Creates a new network with this device as its first member.
    CreateNetwork {
        /// Network display name.
        name: String,
        /// This device's name in the network.
        device_name: String,
    },
    /// Issues an invite link.
    CreateInvite {
        /// Network to invite into.
        network: NetworkId,
        /// Lifetime of the invite.
        expires_in_secs: u64,
        /// How many devices may use it; `None` is unlimited.
        max_uses: Option<u32>,
    },
    /// Joins a network through an invite link.
    Join {
        /// The invite link.
        invite_link: String,
        /// This device's name in the network.
        device_name: String,
    },
    /// Leaves a network.
    Leave {
        /// Network to leave.
        network: NetworkId,
    },
    /// Lists a network's peers.
    Peers {
        /// Network to list.
        network: NetworkId,
    },
    /// Creates or replaces a DNS record.
    SetRecord {
        /// Network owning the record.
        network: NetworkId,
        /// Record name.
        name: String,
        /// Record data.
        record: DnsRecord,
    },
    /// Deletes a DNS record.
    DeleteRecord {
        /// Network owning the record.
        network: NetworkId,
        /// Record name.
        name: String,
    },
    /// Replaces a network's policy.
    SetPolicy {
        /// Network to change.
        network: NetworkId,
        /// The policy document as TOML.
        policy_toml: String,
    },
    /// Routes internet traffic through an exit node, or stops with `None`.
    UseExit {
        /// Network of the exit node.
        network: NetworkId,
        /// The exit node.
        node: Option<NodeId>,
    },
    /// Advertises a relay server as the record `relay/<name>`.
    AdvertiseRelay {
        /// Network to advertise in.
        network: NetworkId,
        /// Relay name.
        name: String,
        /// The relay's `host:port`.
        address: String,
        /// The node running the relay; `None` is this device.
        node: Option<NodeId>,
    },
    /// Removes the record `relay/<name>`.
    RemoveRelay {
        /// Network owning the record.
        network: NetworkId,
        /// Relay name.
        name: String,
    },
    /// Grants or revokes a node's right to act as an exit node.
    SetExitGrant {
        /// Network to change.
        network: NetworkId,
        /// The node.
        node: NodeId,
        /// Grant when true, revoke when false.
        allowed: bool,
    },
    /// Offers this device as an exit node and/or relay.
    ServerMode {
        /// Act as an exit node.
        exit: bool,
        /// Act as a relay.
        relay: bool,
    },
    /// Imports a plain WireGuard configuration as a network.
    ImportWireGuard {
        /// Name for the imported network.
        name: String,
        /// The `wg-quick` configuration text.
        conf_text: String,
    },
    /// Exports a WireGuard configuration for a device without kith.
    ExportWireGuard {
        /// The network to add the device to.
        network: NetworkId,
        /// The name the device asks for.
        device_name: String,
        /// Its policy tags.
        tags: Vec<String>,
    },
    /// Turns the connection into an [`Event`] stream.
    Watch,
    /// Removes a device from a network; admins only.
    RemoveDevice {
        /// Network to change.
        network: NetworkId,
        /// The device.
        node: NodeId,
    },
    /// Lists a network's DNS names: every device's and the custom records.
    Records {
        /// Network to list.
        network: NetworkId,
    },
}

/// A typed DNS record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DnsRecord {
    /// IPv4 address.
    A(Ipv4Addr),
    /// IPv6 address.
    Aaaa(Ipv6Addr),
    /// Alias to another name.
    Cname(String),
    /// Free text.
    Txt(String),
}

/// One DNS name and its data, from [`Request::Records`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordInfo {
    /// The full name, such as `nas.home.kith.internal`.
    pub name: String,
    /// The data.
    pub record: DnsRecord,
    /// Whether kith made it from a device's name, rather than an admin setting it.
    pub device: bool,
}

/// The daemon's answer to one [`Request`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Response {
    /// Accepts [`Request::Hello`].
    Hello {
        /// The daemon's [`crate::PROTOCOL_VERSION`].
        protocol: u32,
        /// The daemon's software version.
        daemon_version: String,
    },
    /// Answers [`Request::Status`].
    Status(Status),
    /// Answers [`Request::CreateNetwork`].
    NetworkCreated {
        /// The new network.
        network: NetworkId,
    },
    /// Answers [`Request::CreateInvite`].
    Invite {
        /// The invite link.
        invite_link: String,
    },
    /// Answers [`Request::Join`].
    Joined {
        /// The joined network.
        network: NetworkId,
        /// This device's name in it, which differs from the one asked for when another
        /// device already holds that name.
        device: String,
    },
    /// Answers [`Request::Leave`].
    Left,
    /// Answers [`Request::Peers`].
    Peers(Vec<PeerInfo>),
    /// Answers [`Request::SetRecord`].
    RecordSet,
    /// Answers [`Request::DeleteRecord`].
    RecordDeleted,
    /// Answers [`Request::SetPolicy`].
    PolicySet,
    /// Answers [`Request::UseExit`].
    ExitSet,
    /// Answers [`Request::AdvertiseRelay`].
    RelayAdvertised {
        /// The address advertised.
        address: String,
    },
    /// Answers [`Request::RemoveRelay`].
    RelayRemoved,
    /// Answers [`Request::SetExitGrant`].
    ExitGrantSet,
    /// Answers [`Request::ServerMode`].
    ServerModeSet {
        /// What was advertised, or what an admin must still run.
        notes: Vec<String>,
    },
    /// Answers [`Request::ImportWireGuard`].
    Imported {
        /// The imported network.
        network: NetworkId,
    },
    /// Answers [`Request::ExportWireGuard`].
    WireGuardConfig {
        /// The device's unique name in the network.
        device: String,
        /// Where the device connects to this one, as `host:port`.
        endpoint: String,
        /// A `wg-quick` config that sends only mesh traffic through kith.
        mesh: String,
        /// The same device's config that sends all its traffic through this one, when
        /// this device is an exit.
        everything: Option<String>,
    },
    /// Answers [`Request::Watch`]; events follow.
    Watching,
    /// The request failed.
    Error {
        /// Machine-readable kind.
        code: ErrorCode,
        /// Human-readable explanation.
        message: String,
    },
    /// Answers [`Request::RemoveDevice`].
    DeviceRemoved,
    /// Answers [`Request::Records`].
    Records(Vec<RecordInfo>),
}

/// Why the daemon refused a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Client and daemon speak different protocol versions.
    VersionMismatch,
    /// The first request was not `Hello`.
    HelloRequired,
    /// The caller may not perform this request.
    PermissionDenied,
    /// The named network, node or record does not exist.
    NotFound,
    /// An argument was invalid.
    InvalidArgument,
    /// The daemon failed internally.
    Internal,
}

/// The daemon's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// This device's identity, once created.
    pub node: Option<NodeId>,
    /// Networks this device belongs to.
    pub networks: Vec<NetworkSummary>,
    /// Whether this device offers itself as an exit node.
    pub exit: bool,
    /// Whether this device offers itself as a relay.
    pub relay: bool,
    /// The exit node in use, as `network/device`.
    pub exit_node: Option<String>,
    /// The WireGuard UDP port; the control channel uses the same TCP port.
    pub listen_port: u16,
    /// Where names outside the mesh are forwarded.
    pub dns_upstreams: Vec<String>,
    /// Packets the dataplane dropped, by reason; only reasons that occurred.
    pub drops: BTreeMap<String, u64>,
    /// Routes other interfaces hold inside kith's ranges, usually another VPN's.
    pub route_conflicts: Vec<String>,
    /// Recent warnings, newest last.
    pub warnings: Vec<Warning>,
}

/// A warning the daemon logged, kept for `kith status` and `kith doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// How long ago it happened.
    pub secs_ago: u64,
    /// What happened.
    pub message: String,
}

/// One network in [`Status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSummary {
    /// Network id.
    pub id: NetworkId,
    /// Display name.
    pub name: String,
    /// Peers online now, as [`PeerInfo::online`] decides.
    pub peers_online: u32,
    /// Total members.
    pub peers_total: u32,
    /// Relays advertised in the network.
    pub relays: u32,
    /// What is wrong with the network's records now, each with how to fix it.
    pub problems: Vec<Problem>,
}

/// Something wrong in a network, such as a phone that lost its address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// What is wrong.
    pub what: String,
    /// How to fix it.
    pub fix: String,
}

/// One peer in [`Response::Peers`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Peer identity.
    pub node: NodeId,
    /// The device's name in the network: unique, and a DNS label.
    pub name: String,
    /// Mesh address, as text.
    pub address: String,
    /// Current path, or `None` while offline.
    pub path: Option<PathKind>,
    /// Why that path was chosen, such as which addresses failed to answer.
    pub path_reason: Option<String>,
    /// Seconds since anything authenticated last arrived from it, remembered across
    /// restarts, or `None` if it has never been heard from.
    pub last_seen_secs: Option<u64>,
    /// The peer's current WireGuard endpoint, as text.
    pub endpoint: Option<String>,
    /// Bytes received from the peer.
    pub rx_bytes: u64,
    /// Bytes sent to the peer.
    pub tx_bytes: u64,
    /// Whether the peer is reachable now: something authenticated arrived from it
    /// recently (for a phone, at its gateway). Everything that shows liveness uses this.
    pub online: bool,
    /// The name the device asked for, when its unique `name` had to differ from it.
    pub display_name: String,
    /// For a device on plain WireGuard, such as a phone, the unique name of the kith
    /// device it connects through.
    pub gateway: Option<String>,
    /// The largest packet the path to it carries, when smaller than the tunnel MTU. Bigger
    /// TCP is kept smaller, IPv4 split, and senders of other traffic told to shrink.
    pub path_mtu: Option<u16>,
}

/// How traffic reaches a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PathKind {
    /// Peer-to-peer UDP.
    Direct,
    /// Through a relay.
    Relay,
    /// Through an obfuscated transport.
    Stealth,
}

/// A change pushed to a watching client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Event {
    /// A peer became reachable.
    PeerConnected {
        /// Network of the peer.
        network: NetworkId,
        /// The peer.
        node: NodeId,
        /// Path in use.
        path: PathKind,
    },
    /// A peer became unreachable.
    PeerDisconnected {
        /// Network of the peer.
        network: NetworkId,
        /// The peer.
        node: NodeId,
    },
    /// Membership, policy or names of a network changed.
    NetworkChanged {
        /// The network.
        network: NetworkId,
    },
    /// Something the user should know about.
    Warning {
        /// Human-readable text.
        message: String,
    },
}

/// Everything the server sends: responses and, after `Watch`, events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ServerMessage {
    Response(Response),
    Event(Event),
}
