use std::collections::BTreeMap;

use core::net::{Ipv4Addr, Ipv6Addr};

use kith_types::{AccountId, NetworkId, NodeId};
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
    /// Sends the journal from entry `since` on (the oldest kept, if `None` or too old),
    /// then each new entry as it happens, until the connection closes.
    Journal {
        /// The first sequence number wanted.
        since: Option<u64>,
    },
    /// Traffic per peer over the last hour.
    Traffic,
    /// Every device's latest report of the peers it reaches in a network.
    Topology {
        /// Network to map.
        network: NetworkId,
    },
    /// Makes a code a new device can use to ask an admin to add it. Any member may.
    RequestCode {
        /// The network.
        network: NetworkId,
    },
    /// Asks, through a request code, for an admin to add this device. Send it again to
    /// learn whether an admin approved or denied the request.
    RequestJoin {
        /// The `kith://request/` code.
        request_code: String,
        /// This device's name in the network.
        device_name: String,
    },
    /// Lists the join requests waiting for an admin, in every network.
    PendingJoins,
    /// Adds a device that asked to join; admins only.
    ApproveJoin {
        /// The network.
        network: NetworkId,
        /// The device.
        node: NodeId,
    },
    /// Turns down a device that asked to join; admins only.
    DenyJoin {
        /// The network.
        network: NetworkId,
        /// The device.
        node: NodeId,
    },
    /// Recent inbound connections a network's policy refused.
    Blocked {
        /// Network to list.
        network: NetworkId,
    },
    /// Adds the one policy rule that allows a blocked connection; admins only.
    AllowBlocked {
        /// Network to change.
        network: NetworkId,
        /// The device that tried to connect.
        from: NodeId,
        /// The device it tried to reach.
        to: NodeId,
        /// `tcp`, `udp`, `icmp` or `any`.
        protocol: String,
        /// The destination port, for TCP and UDP.
        port: Option<u16>,
    },
    /// Renames a device in a network; admins only.
    RenameDevice {
        /// Network to change.
        network: NetworkId,
        /// The device.
        node: NodeId,
        /// Its new name.
        name: String,
    },
    /// Lists a network's invites that can still admit devices.
    ListInvites {
        /// Network to list.
        network: NetworkId,
    },
    /// Revokes an invite for good; admins only.
    RevokeInvite {
        /// Network owning the invite.
        network: NetworkId,
        /// The invite's id, as [`InviteInfo::id`] gives it.
        invite: String,
    },
    /// Reads a network's policy.
    GetPolicy {
        /// Network to read.
        network: NetworkId,
    },
    /// Lists the relays advertised in a network.
    ListRelays {
        /// Network to list.
        network: NetworkId,
    },
    /// Lists the devices an admin allowed to act as exit nodes in a network.
    ListExits {
        /// Network to list.
        network: NetworkId,
    },
    /// Starts or stops sending traffic through an imported WireGuard network.
    UseImport {
        /// The imported network, as [`ImportedInfo::network`] gives it.
        network: NetworkId,
        /// Send its routes through it when true; leave it configured but idle when false.
        active: bool,
    },
    /// Removes an imported WireGuard network and forgets its config.
    RemoveImport {
        /// The imported network.
        network: NetworkId,
    },
    /// Lists a network's people, its authorized consoles, and whether it has a recovery key.
    Accounts {
        /// Network to list.
        network: NetworkId,
    },
    /// Adds a network's first person, as its owner, signed by this device as an admin.
    /// Valid only while the network has no people; everyone after needs an approval
    /// through [`Request::PrepareAuthority`].
    AddAccount {
        /// The network.
        network: NetworkId,
        /// The person's name.
        name: String,
        /// Their role; must be [`AccountRole::Owner`].
        role: AccountRole,
        /// Their first passkey, whose key names the account.
        credential: PasskeyCredential,
    },
    /// Records whose device `device` is; admins only, or a console acting for one.
    AssignDevice {
        /// The network.
        network: NetworkId,
        /// The device.
        device: NodeId,
        /// The person it belongs to.
        account: AccountId,
    },
    /// Drafts a change to people or consoles, authored by this device, for a passkey (or
    /// the recovery words) to approve. Nothing changes until it is submitted.
    PrepareAuthority {
        /// The network.
        network: NetworkId,
        /// The change.
        change: AuthorityChange,
    },
    /// Publishes a change from [`Response::AuthorityPrepared`] with a passkey's assertion of
    /// its challenge. Refused, and not published, unless the network accepts it.
    SubmitAuthority {
        /// The network.
        network: NetworkId,
        /// `op_bytes` from [`Response::AuthorityPrepared`], unchanged.
        op_bytes: Vec<u8>,
        /// The passkey's assertion.
        assertion: PasskeyAssertion,
    },
    /// Publishes a change from [`Response::AuthorityPrepared`] approved by the network's
    /// recovery key, which counts as an owner for adding people and authorizing consoles.
    RecoverAuthority {
        /// The network.
        network: NetworkId,
        /// `op_bytes` from [`Response::AuthorityPrepared`], unchanged.
        op_bytes: Vec<u8>,
        /// The recovery words.
        words: String,
    },
    /// Creates the network's recovery key and returns it once as words; the network keeps
    /// only its public half. Valid only before the network has any people, so ask for it
    /// before the first [`Request::AddAccount`].
    RecoveryKey {
        /// The network.
        network: NetworkId,
    },
    /// Runs an admin request on behalf of a person: every change it makes names `actor`,
    /// and is valid only while `actor` is an owner or admin. Lets an authorized console
    /// that is not an admin device make everyday changes.
    AsActor {
        /// The person who asked.
        actor: AccountId,
        /// The request; not itself an `AsActor`.
        request: Box<Self>,
    },
    /// Lists the devices a plain WireGuard device can connect through, default first.
    Gateways {
        /// The network.
        network: NetworkId,
    },
    /// Like [`Request::ExportWireGuard`], through a chosen gateway.
    ExportWireGuardVia {
        /// The network to add the device to.
        network: NetworkId,
        /// The name the device asks for.
        device_name: String,
        /// Its policy tags.
        tags: Vec<String>,
        /// The gateway; `None` picks the default from [`Request::Gateways`].
        gateway: Option<NodeId>,
    },
    /// Stops carrying traffic, with routes and DNS taken back, while the daemon keeps
    /// syncing; for `for_secs` seconds, or until [`Request::Resume`].
    Pause {
        /// How long; `None` until resumed.
        for_secs: Option<u64>,
    },
    /// Carries traffic again after [`Request::Pause`].
    Resume,
    /// Whether the tunnel is paused.
    PauseState,
}

/// A person's role in a network, from [`Request::Accounts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum AccountRole {
    /// Everything, including people's roles and consoles.
    Owner,
    /// Everyday changes, and adding people other than owners.
    Admin,
    /// Uses the network; changes nothing.
    Member,
}

/// A passkey as registered: what WebAuthn's `create()` returned, in the ledger's terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyCredential {
    /// The credential id (WebAuthn's raw id).
    pub credential_id: Vec<u8>,
    /// The P-256 public key as an uncompressed SEC1 point: `0x04 || x || y`, 65 bytes.
    pub public_key: Vec<u8>,
    /// The relying party id it was created for, such as `vpn.example.com`.
    pub rp_id: String,
}

/// A passkey's answer to WebAuthn's `get()` with a challenge from
/// [`Response::AuthorityPrepared`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyAssertion {
    /// The account the passkey belongs to.
    pub account: AccountId,
    /// The credential id (WebAuthn's raw id).
    pub credential_id: Vec<u8>,
    /// `authenticatorData`, unchanged.
    pub authenticator_data: Vec<u8>,
    /// `clientDataJSON`, unchanged.
    pub client_data_json: Vec<u8>,
    /// `signature`: ES256, DER-encoded, unchanged.
    pub signature: Vec<u8>,
}

/// A change to people or consoles, for [`Request::PrepareAuthority`]. Owners may make any;
/// admins may add and remove people other than owners and revoke consoles; the recovery
/// key may add people and authorize consoles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorityChange {
    /// Adds a person.
    AddAccount {
        /// Their name.
        name: String,
        /// Their role.
        role: AccountRole,
        /// Their first passkey.
        credential: PasskeyCredential,
    },
    /// Removes a person and every device assigned to them. The last owner stays.
    RemoveAccount {
        /// The person.
        account: AccountId,
    },
    /// Changes a person's role; owners only. The last owner stays one.
    SetRole {
        /// The person.
        account: AccountId,
        /// Their new role.
        role: AccountRole,
    },
    /// Lets a device sign everyday changes on behalf of people: the Console's device.
    AuthorizeConsole {
        /// The device.
        device: NodeId,
    },
    /// Takes that power away again.
    RevokeConsole {
        /// The device.
        device: NodeId,
    },
    /// Adds another passkey to a person; their own passkey or an owner's approves it.
    AddCredential {
        /// The person.
        account: AccountId,
        /// The new passkey.
        credential: PasskeyCredential,
    },
}

/// A network's people, from [`Request::Accounts`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountsInfo {
    /// Everyone, by name.
    pub accounts: Vec<AccountInfo>,
    /// The devices authorized to act for people.
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub consoles: Vec<NodeId>,
    /// Whether the network has a recovery key.
    pub recovery_set: bool,
}

/// One person in [`AccountsInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountInfo {
    /// The account.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub id: AccountId,
    /// Their name.
    pub name: String,
    /// Their role.
    pub role: AccountRole,
    /// Their passkeys.
    pub credentials: Vec<CredentialInfo>,
    /// The member devices assigned to them.
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub devices: Vec<NodeId>,
}

/// One passkey of an [`AccountInfo`], enough to check a login assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CredentialInfo {
    /// The credential id.
    pub credential_id: Vec<u8>,
    /// The P-256 public key, uncompressed SEC1.
    pub public_key: Vec<u8>,
    /// The relying party id.
    pub rp_id: String,
}

/// An invite that can still admit devices, from [`Request::ListInvites`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InviteInfo {
    /// The invite's public key in lowercase hex: its id for [`Request::RevokeInvite`].
    pub id: String,
    /// When it stops admitting devices, in milliseconds since the Unix epoch.
    pub expires_at_ms: u64,
    /// How many more devices it admits; `None` is unlimited.
    pub uses_left: Option<u32>,
    /// When it was created, in milliseconds since the Unix epoch.
    pub created_at_ms: u64,
}

/// A network's policy, from [`Request::GetPolicy`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PolicyInfo {
    /// The policy as the TOML [`Request::SetPolicy`] takes.
    pub toml: String,
    /// The same rules, structured.
    pub rules: Vec<PolicyRuleInfo>,
}

/// One allow rule of a [`PolicyInfo`]: any source may reach any destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PolicyRuleInfo {
    /// Which devices may send.
    pub sources: Vec<SelectorInfo>,
    /// What they may reach.
    pub destinations: Vec<TargetInfo>,
}

/// Which devices a policy selector names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SelectorInfo {
    /// `any`, `device` or `tag`.
    pub kind: String,
    /// The device id or tag name; `None` for `any`.
    pub value: Option<String>,
    /// The device's name in the network, when the selector names a known device.
    pub name: Option<String>,
}

/// One destination of a [`PolicyRuleInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TargetInfo {
    /// The devices.
    pub selector: SelectorInfo,
    /// `any`, `tcp`, `udp` or `icmp`.
    pub protocol: String,
    /// The ports opened, as `start-end` ranges or single ports; empty means every port.
    pub ports: Vec<String>,
}

/// A relay advertised in a network, from [`Request::ListRelays`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RelayInfo {
    /// The relay's name.
    pub name: String,
    /// Its `host:port`.
    pub address: String,
    /// The node running it.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub node: NodeId,
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
    /// Answers [`Request::Journal`]; entries follow on the connection.
    Journaling {
        /// The sequence number the next new entry will get: entries before it are history.
        next: u64,
    },
    /// Answers [`Request::Traffic`].
    Traffic(Vec<TrafficSeries>),
    /// Answers [`Request::Topology`].
    Topology(Vec<LinkReport>),
    /// Answers [`Request::RequestCode`].
    RequestCode {
        /// The `kith://request/` code.
        code: String,
    },
    /// Answers [`Request::RequestJoin`]: no admin has decided yet.
    JoinPending {
        /// The network.
        network: NetworkId,
        /// Its name, as the code gives it.
        network_name: String,
    },
    /// Answers [`Request::RequestJoin`]: an admin denied the request.
    JoinDenied {
        /// The network.
        network: NetworkId,
    },
    /// Answers [`Request::PendingJoins`].
    PendingJoins(Vec<PendingJoin>),
    /// Answers [`Request::ApproveJoin`].
    JoinApproved {
        /// The device's name in the network.
        device: String,
    },
    /// Answers [`Request::DenyJoin`].
    JoinRequestDenied,
    /// Answers [`Request::Blocked`], most recent first.
    Blocked(Vec<BlockedInfo>),
    /// Answers [`Request::AllowBlocked`].
    BlockedAllowed,
    /// Answers [`Request::RenameDevice`].
    DeviceRenamed,
    /// Answers [`Request::ListInvites`].
    Invites(Vec<InviteInfo>),
    /// Answers [`Request::RevokeInvite`].
    InviteRevoked,
    /// Answers [`Request::GetPolicy`].
    Policy(PolicyInfo),
    /// Answers [`Request::ListRelays`].
    Relays(Vec<RelayInfo>),
    /// Answers [`Request::ListExits`].
    Exits(Vec<NodeId>),
    /// Answers [`Request::UseImport`].
    ImportUsed,
    /// Answers [`Request::RemoveImport`].
    ImportRemoved,
    /// Answers [`Request::Accounts`].
    Accounts(AccountsInfo),
    /// Answers [`Request::AddAccount`].
    AccountAdded {
        /// The new account.
        account: AccountId,
    },
    /// Answers [`Request::AssignDevice`].
    DeviceAssigned,
    /// Answers [`Request::PrepareAuthority`].
    AuthorityPrepared {
        /// The drafted op, to send back with its approval.
        op_bytes: Vec<u8>,
        /// What the passkey signs: the WebAuthn `challenge` as unpadded base64url. It is
        /// also exactly what the assertion's client data must carry.
        challenge: String,
    },
    /// Answers [`Request::SubmitAuthority`] and [`Request::RecoverAuthority`]: the change
    /// took effect.
    AuthorityApplied,
    /// Answers [`Request::RecoveryKey`]. Shown once: nothing else keeps the words.
    RecoveryKey {
        /// The recovery code, as space-separated words.
        words: String,
    },
    /// Answers [`Request::Gateways`]: the default first.
    Gateways(Vec<GatewayInfo>),
    /// Answers [`Request::Pause`], [`Request::Resume`] and [`Request::PauseState`].
    PauseState {
        /// Whether traffic is paused.
        paused: bool,
        /// Seconds until it resumes on its own, when paused for a while.
        resume_in_secs: Option<u64>,
    },
}

/// A device that plain WireGuard devices can connect through, from [`Request::Gateways`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GatewayInfo {
    /// The device.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub node: NodeId,
    /// Its name in the network.
    pub name: String,
    /// Where phones reach it, as `host:port`.
    pub endpoint: String,
    /// Whether it is an exit, so phones may send all traffic through it.
    pub exit: bool,
    /// Whether it is the one picked when none is chosen.
    pub default: bool,
}

/// One device's report of the peers it reaches, from [`Response::Topology`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinkReport {
    /// The reporting device.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub node: NodeId,
    /// Its name in the network.
    pub name: String,
    /// Seconds since the report was received; 0 for this device's own.
    pub age_secs: u64,
    /// The peers it reaches.
    pub links: Vec<LinkInfo>,
}

/// One peer in a [`LinkReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinkInfo {
    /// The peer reached.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub peer: NodeId,
    /// Its name in the network, or its short id if the reporter knows a peer this device
    /// does not.
    pub peer_name: String,
    /// The path in use.
    pub path: PathKind,
    /// The relay carrying the traffic, when relayed.
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub relay: Option<NodeId>,
    /// The measured round-trip time in milliseconds, once known.
    pub rtt_ms: Option<u32>,
    /// Bytes per second both ways over the last sample.
    pub bytes_per_sec: u64,
}

/// A device waiting for an admin to let it join, from [`Request::PendingJoins`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PendingJoin {
    /// The network it asked to join.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub network: NetworkId,
    /// The device.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub node: NodeId,
    /// The name it asked for.
    pub name: String,
    /// Where it asked from, when it asked this device directly.
    pub from: Option<String>,
    /// Seconds since it asked.
    pub age_secs: u64,
}

/// A connection a network's policy refused, from [`Request::Blocked`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BlockedInfo {
    /// The network.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub network: NetworkId,
    /// The device that tried to connect.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub from: NodeId,
    /// Its name in the network.
    pub from_name: String,
    /// The device it tried to reach.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub to: NodeId,
    /// Its name in the network.
    pub to_name: String,
    /// `tcp`, `udp`, `icmp`, or the IP protocol number.
    pub protocol: String,
    /// The destination port, for TCP and UDP.
    pub port: Option<u16>,
    /// Packets refused; under a flood only a sample is counted.
    pub count: u64,
    /// When the latest was refused, in milliseconds since the Unix epoch.
    pub last_ms: u64,
    /// The policy rule, as TOML, that would allow just this connection.
    pub rule: String,
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Status {
    /// This device's identity, once created.
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
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
    /// Plain WireGuard networks imported from a `wg-quick` config.
    pub imported: Vec<ImportedInfo>,
}

/// A plain WireGuard network imported from a `wg-quick` config, in [`Status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportedInfo {
    /// The id it is addressed by in [`Request::UseImport`] and [`Request::RemoveImport`].
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub network: NetworkId,
    /// The name it was imported under.
    pub name: String,
    /// Where its server is reached, as text.
    pub endpoint: Option<String>,
    /// What it carries, as prefixes.
    pub routes: Vec<String>,
    /// Whether it carries all internet traffic, so acts as an exit.
    pub everything: bool,
    /// Whether its routes go through it now.
    pub active: bool,
    /// Seconds since anything arrived from its server, if ever.
    pub last_seen_secs: Option<u64>,
    /// Bytes received from it.
    pub rx_bytes: u64,
    /// Bytes sent to it.
    pub tx_bytes: u64,
}

/// A warning the daemon logged, kept for `kith status` and `kith doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Warning {
    /// How long ago it happened.
    pub secs_ago: u64,
    /// What happened.
    pub message: String,
}

/// One network in [`Status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NetworkSummary {
    /// Network id.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
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
    /// Whether this device is an admin of the network, so may change it.
    pub admin: bool,
}

/// Something wrong in a network, such as a phone that lost its address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Problem {
    /// What is wrong.
    pub what: String,
    /// How to fix it.
    pub fix: String,
}

/// One peer in [`Response::Peers`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PeerInfo {
    /// Peer identity.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
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
    /// The invite the device joined with, as its [`InviteInfo::id`]; `None` for a device
    /// added another way, such as by an admin, an approved request or as a phone.
    pub invite: Option<String>,
}

/// How traffic reaches a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[non_exhaustive]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
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
    Entry(Entry),
}

/// One thing that happened, numbered so a client can resume where it stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Entry {
    /// Increases by one per entry, from 0 when the daemon started.
    pub seq: u64,
    /// When, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// What.
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub kind: EntryKind,
}

/// What a journal [`Entry`] records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[non_exhaustive]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "snake_case"))]
pub enum EntryKind {
    /// A peer came online, reached as `path` (or through `gateway`, for a phone).
    PeerOnline {
        /// Its network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The peer.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        node: NodeId,
        /// Its name in the network.
        name: String,
        /// How it is reached.
        path: Option<PathKind>,
        /// The kith device a plain WireGuard member goes through.
        gateway: Option<String>,
    },
    /// A peer went offline.
    PeerOffline {
        /// Its network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The peer.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        node: NodeId,
        /// Its name in the network.
        name: String,
    },
    /// An online peer is now reached another way.
    PathChanged {
        /// Its network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The peer.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        node: NodeId,
        /// Its name in the network.
        name: String,
        /// The path before.
        from: Option<PathKind>,
        /// The path now.
        to: Option<PathKind>,
    },
    /// An online peer's address changed, as when a phone moves from Wi-Fi to mobile data.
    EndpointChanged {
        /// Its network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The peer.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        node: NodeId,
        /// Its name in the network.
        name: String,
        /// The new address.
        endpoint: String,
    },
    /// A device became a member.
    DeviceJoined {
        /// The network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The device.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        node: NodeId,
        /// Its name.
        name: String,
        /// Its mesh address.
        address: String,
        /// The invite it joined with, as its [`InviteInfo::id`]; `None` for a device
        /// added another way.
        invite: Option<String>,
    },
    /// A device stopped being a member.
    DeviceRemoved {
        /// The network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The device.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        node: NodeId,
        /// Its name.
        name: String,
    },
    /// The exit node this device uses changed; `None` when it stopped using one.
    ExitChanged {
        /// `network/device`.
        exit: Option<String>,
    },
    /// The data plane dropped more packets for `reason`.
    Dropped {
        /// Why, as `kith status` names it.
        reason: String,
        /// How many since the last entry for this reason.
        count: u64,
    },
    /// A problem the daemon hit.
    Warning {
        /// Human-readable text.
        message: String,
    },
    /// A line from the daemon's log.
    Log {
        /// `error`, `warn`, `info`, `debug` or `trace`.
        level: String,
        /// The module that logged it.
        target: String,
        /// The message and its fields.
        message: String,
    },
    /// A device asked to join and waits for an admin.
    JoinRequested {
        /// The network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The device.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        node: NodeId,
        /// The name it asked for.
        name: String,
    },
    /// The network's policy refused a connection; at most one entry per connection every
    /// 30 seconds.
    Blocked {
        /// The network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The device that tried to connect.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        from: NodeId,
        /// Its name in the network.
        from_name: String,
        /// The device it tried to reach.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        to: NodeId,
        /// Its name in the network.
        to_name: String,
        /// `tcp`, `udp`, `icmp`, or the IP protocol number.
        protocol: String,
        /// The destination port, for TCP and UDP.
        port: Option<u16>,
        /// Packets refused since the last entry for this connection.
        count: u64,
    },
    /// A device's name in the network changed.
    DeviceRenamed {
        /// The network.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        network: NetworkId,
        /// The device.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        node: NodeId,
        /// Its name before.
        old_name: String,
        /// Its name now.
        name: String,
    },
}

/// Traffic counters of one peer, sampled every few seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TrafficSeries {
    /// Its network.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub network: NetworkId,
    /// The peer.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub node: NodeId,
    /// Its name in the network.
    pub name: String,
    /// Oldest first.
    pub samples: Vec<TrafficSample>,
}

/// Total bytes received from and sent to a peer at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TrafficSample {
    /// When, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// Received since the session began.
    pub rx_bytes: u64,
    /// Sent since the session began.
    pub tx_bytes: u64,
}
