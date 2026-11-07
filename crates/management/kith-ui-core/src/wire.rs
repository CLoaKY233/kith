//! The JSON the page reads and writes, beyond the daemon's own types.
//!
//! These types are the contract in `docs/plans/kith-ui-api.md`. Fields that hold a
//! `kith-ipc` type are kept as JSON already reshaped by [`crate::json`], so the TypeScript
//! side sees the `kith-ipc` declaration and the page sees ids as hex.

mod console;

use std::collections::BTreeMap;

pub use console::*;
use kith_ipc::PathKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Versions of this server and of the daemon it talks to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Versions {
    /// `kith-ui`'s version.
    pub ui: String,
    /// The daemon's version, empty while it is unreachable.
    pub daemon: String,
    /// Whether the two speak the same IPC protocol.
    pub compatible: bool,
}

/// This device's standing in a network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum Role {
    /// Signs changes to the network.
    Admin,
    /// Sees the network; changes need an admin.
    Member,
    /// Sees what the policy allows and nothing more.
    ViewOnly,
    /// Was removed; the network is shown as it was.
    Removed,
}

/// This device in one network: the peers list never includes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Membership {
    /// Network id.
    pub network: String,
    /// This device's name in the network.
    pub name: String,
    /// This device's mesh address.
    pub address: String,
    /// Its standing.
    pub role: Role,
    /// Its key's short fingerprint, as `k7f3·9a21`.
    pub key_fpr: String,
    /// The device that made it admin, when it is one and was not the founder.
    pub made_admin_by: Option<String>,
    /// When it was made admin, in Unix milliseconds.
    pub made_admin_at_ms: Option<u64>,
    /// Every admin's name, for "Changes need an admin: desk or nas".
    pub admins: Vec<String>,
    /// Whether this device shares its link reports with the network.
    pub share_links: bool,
}

/// The computer itself, for Welcome before any network exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Host {
    /// The host name, as a suggested device name.
    pub name: String,
    /// The operating system, such as `macOS` or `Linux`.
    pub os: String,
}

/// How the exit in use is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum ExitState {
    /// Internet traffic goes through it.
    Active,
    /// It is offline and internet traffic is blocked.
    OfflineBlocked,
    /// It is offline and internet traffic goes directly.
    OfflineDirect,
}

/// `GET /api/status`: the daemon's `Status` plus what only the page needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(as = "StatusViewDecl"))]
pub struct StatusView {
    /// The daemon's status, reshaped.
    #[serde(flatten)]
    pub status: Value,
    /// Whether this OS user may change kith on this device.
    pub operator: bool,
    /// Versions of `kith-ui` and the daemon.
    pub versions: Versions,
    /// When a pause ends, in Unix milliseconds; `null` when not paused.
    pub paused_until: Option<u64>,
    /// This device in each network.
    pub memberships: Vec<Membership>,
    /// The computer.
    pub host: Host,
    /// How the exit in use is doing; `null` without one.
    pub exit_state: Option<ExitState>,
    /// How this server is reached.
    #[serde(default)]
    pub mode: ConsoleMode,
    /// The signed-in person; `null` for the local link's session or no session.
    #[serde(default)]
    pub me: Option<Me>,
}

/// [`StatusView`] with its `Status` typed, for its TypeScript declaration: ts-rs cannot
/// flatten a field it also retypes.
#[cfg(feature = "ts")]
#[doc(hidden)]
#[derive(Debug, ts_rs::TS)]
#[ts(rename = "StatusView")]
pub struct StatusViewDecl {
    /// The daemon's status.
    #[ts(flatten)]
    pub status: kith_ipc::Status,
    /// Whether this OS user may change kith on this device.
    pub operator: bool,
    /// Versions of `kith-ui` and the daemon.
    pub versions: Versions,
    /// When a pause ends, in Unix milliseconds; `null` when not paused.
    pub paused_until: Option<u64>,
    /// This device in each network.
    pub memberships: Vec<Membership>,
    /// The computer.
    pub host: Host,
    /// How the exit in use is doing; `null` without one.
    pub exit_state: Option<ExitState>,
    /// How this server is reached.
    pub mode: ConsoleMode,
    /// The signed-in person; `null` for the local link's session or no session.
    pub me: Option<Me>,
}

/// One exit in a network, for the Exits screen and the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExitView {
    /// Network id.
    pub network: String,
    /// The exit device.
    pub node: String,
    /// Its name.
    pub name: String,
    /// Whether this device may use it.
    pub allowed: bool,
    /// Names of devices allowed to use it; empty when all are.
    pub access: Vec<String>,
    /// Names of devices using it now.
    pub in_use_by: Vec<String>,
}

/// How busy a link is, shown to members instead of another device's exact rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum RateBand {
    /// Under 1 kB/s.
    Idle,
    /// Up to 1 MB/s.
    Light,
    /// Above 1 MB/s.
    Busy,
}

/// One device's traffic counters at a tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TrafficTick {
    /// Network id.
    pub network: String,
    /// Device id.
    pub node: String,
    /// Bytes received from it since its session began.
    pub rx: u64,
    /// Bytes sent to it since its session began.
    pub tx: u64,
}

/// One link as a device reported it, for the map, paths and latency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinkTick {
    /// Network id.
    pub network: String,
    /// The reporting device.
    pub from: String,
    /// The device it reaches.
    pub to: String,
    /// The path in use.
    #[serde(with = "crate::json::path_kind")]
    pub path: PathKind,
    /// The relay carrying it, when relayed.
    pub relay: Option<String>,
    /// Round-trip time in milliseconds, once measured.
    pub rtt_ms: Option<u32>,
    /// Share of probes lost, from 0 to 1, once measured.
    pub loss: Option<f32>,
    /// Bytes per second both ways; only for this device's own links or an admin.
    pub rate: Option<u64>,
    /// How busy the link is, when `rate` is withheld.
    pub rate_band: Option<RateBand>,
    /// Seconds since the report arrived; 0 for this device's own.
    pub age_secs: u64,
}

/// `event: tick`, every 2 s: numbers for every device and link. Never replayed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Tick {
    /// When the numbers were read, in Unix milliseconds.
    pub at_ms: u64,
    /// Counters per device.
    pub traffic: Vec<TrafficTick>,
    /// Links per reporting device.
    pub links: Vec<LinkTick>,
}

/// `event: reset`: the daemon restarted, so the page refetches everything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Reset {
    /// The new boot id.
    pub boot: String,
    /// When the daemon started, in Unix milliseconds.
    pub started_at_ms: u64,
}

/// `event: gap`: entries the page asked for have left the daemon's ring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Gap {
    /// The first missing sequence number.
    pub missed_from: u64,
    /// The last missing sequence number.
    pub missed_to: u64,
}

/// What a join request says about itself; shown as a claim, never as fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Claimed {
    /// The name it asked for.
    pub name: String,
    /// The operating system it says it runs.
    pub os: Option<String>,
}

/// What this device saw of a join request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Seen {
    /// The address it asked from, when it asked this device directly.
    pub ip: Option<String>,
    /// Its key's short fingerprint, as `k2c1·77f0`.
    pub key_fpr: String,
}

/// A device waiting for an admin to let it join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JoinRequest {
    /// Stable id, used in `?need=`.
    pub id: String,
    /// Network id.
    pub network: String,
    /// The requesting device's id.
    pub node: String,
    /// Six Crockford base32 characters from a hash of its key; the device shows the same.
    pub check_code: String,
    /// What it says about itself.
    pub claimed: Claimed,
    /// What this device saw.
    pub seen: Seen,
    /// When it asked, in Unix milliseconds.
    pub requested_at_ms: u64,
    /// When the request lapses, in Unix milliseconds.
    pub expires_at_ms: u64,
}

/// Connections the policy refused, grouped by source, destination and port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BlockedGroup {
    /// Stable id, used in `?need=` and `/api/blocked/:id/allow`.
    pub id: String,
    /// Network id.
    pub network: String,
    /// The device that tried to connect.
    pub from: String,
    /// Its name.
    pub from_name: String,
    /// The device it tried to reach.
    pub to: String,
    /// Its name.
    pub to_name: String,
    /// `tcp`, `udp`, `icmp`, or the IP protocol number.
    pub protocol: String,
    /// The destination port, for TCP and UDP.
    pub port: Option<u16>,
    /// Connections refused in the window.
    pub count: u64,
    /// The first refusal in the window, in Unix milliseconds.
    pub first_ms: u64,
    /// The latest refusal, in Unix milliseconds.
    pub last_ms: u64,
    /// The policy rule, as TOML, that would allow just this.
    pub rule: String,
}

/// `GET /api/pending`: everything waiting for a person.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Pending {
    /// Join requests, oldest first.
    pub joins: Vec<JoinRequest>,
    /// Blocked connections, most recent first.
    pub blocked: Vec<BlockedGroup>,
}

/// The snapshot inlined into `index.html` as `<script type="application/json" id="boot">`
/// and served by `GET /api/boot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Boot {
    /// Identifies the daemon's run; a stream entry from another boot means refetch.
    pub boot: String,
    /// The last journal sequence number this snapshot includes.
    pub seq: u64,
    /// The server's clock when the snapshot was taken, for the page's clock offset.
    pub server_time_ms: u64,
    /// Status of the daemon and its networks.
    pub status: StatusView,
    /// Devices per network id.
    #[cfg_attr(feature = "ts", ts(as = "BTreeMap<String, Vec<kith_ipc::PeerInfo>>"))]
    pub peers: BTreeMap<String, Value>,
    /// The last 150 traffic samples per device.
    #[cfg_attr(feature = "ts", ts(as = "Vec<kith_ipc::TrafficSeries>"))]
    pub traffic_tail: Value,
    /// Everything waiting for a person.
    pub pending: Pending,
    /// Versions of `kith-ui` and the daemon.
    pub versions: Versions,
    /// The latest tick, so latency and links show on the first frame.
    pub last_tick: Option<Tick>,
}

/// The type of a name record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "UPPERCASE"))]
pub enum RecordType {
    /// IPv4 address.
    A,
    /// IPv6 address.
    Aaaa,
    /// Another name.
    Cname,
    /// Free text.
    Txt,
}

/// One name in a network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecordView {
    /// The full name, such as `nas.home.kith.internal`.
    pub name: String,
    /// The record type.
    #[serde(rename = "type")]
    #[cfg_attr(feature = "ts", ts(rename = "type"))]
    pub kind: RecordType,
    /// The data.
    pub value: String,
    /// Whether kith made it from a device's name.
    pub device: bool,
    /// The device that signed it, for names an admin set.
    pub set_by: Option<String>,
}

/// Why a request failed, as the page maps it to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum ApiErrorCode {
    /// No session, or the session ended.
    Unauthorized,
    /// The one-time link (or setup link, registration link, device code) was already used.
    SessionUsed,
    /// The one-time link is older than 2 minutes, or another one-time link or code lapsed.
    SessionExpired,
    /// Wrong `Host` or `Origin`.
    Forbidden,
    /// This device may not do this, such as a member changing the network.
    PermissionDenied,
    /// The network, device or name no longer exists.
    NotFound,
    /// Someone else changed it first; `conflict` says who.
    Conflict,
    /// An argument was invalid.
    InvalidArgument,
    /// The daemon is not running.
    DaemonDown,
    /// The daemon runs but this OS user may not talk to it.
    NotInGroup,
    /// `kith-ui` and the daemon speak different protocol versions.
    VersionSkew,
    /// The person at this computer refused or cancelled the OS confirmation.
    OsAuthDenied,
    /// Too many tries; wait a minute.
    RateLimited,
    /// Anything else; `message` says what.
    Internal,
}

/// How a join request was settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum Resolution {
    /// An admin let it in.
    Approved,
    /// An admin turned it down.
    Denied,
    /// Nobody answered in time.
    Expired,
}

/// Who won a race, sent with a 409.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Conflict {
    /// The device that changed it first.
    pub resolved_by: String,
    /// For a join request, what that device decided.
    pub resolution: Option<Resolution>,
    /// When, in Unix milliseconds.
    pub at_ms: u64,
    /// The conflicting operation, for a change made with `based_on`.
    pub op: Option<String>,
}

/// The body of every non-2xx answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApiErrorBody {
    /// What kind of failure.
    pub code: ApiErrorCode,
    /// The daemon's or server's words, for logs and the `Internal` copy.
    pub message: String,
    /// Who won, for a 409.
    pub conflict: Option<Conflict>,
}

/// Where a change stands when its request returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum OutcomeState {
    /// Signed and in this device's ledger.
    Accepted,
    /// Waiting for the person at this computer to confirm with the OS.
    AwaitingOsAuth,
}

/// The answer to a change: its state and, for a ledger operation, the op id that later
/// `op_seen` entries refer to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Outcome {
    /// Where it stands.
    pub state: OutcomeState,
    /// The ledger operation, when the change signed one.
    pub op: Option<String>,
}

/// `POST /api/session`: trades the one-time link token for a session cookie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionIn {
    /// The token from the link's `#t=` fragment.
    pub token: String,
}

/// `POST /api/networks`: starts a network with this device as its first admin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateNetworkIn {
    /// The network's name.
    pub name: String,
    /// This device's name in it.
    pub device_name: String,
}

/// `POST /api/join`: joins through an invite link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JoinIn {
    /// The invite link.
    pub invite_link: String,
    /// This device's name in the network.
    pub device_name: String,
}

/// `POST /api/join/request`: asks an admin to let this device in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestJoinIn {
    /// The network and admin, as `home@studio-mac`, or a `kith://request/` code.
    pub request_code: String,
    /// This device's name in the network.
    pub device_name: String,
}

/// `GET /api/networks/:net/request-code`: what a new device runs to ask an admin to let
/// it in, for "Approve it live".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JoinCode {
    /// The `kith://request/` code.
    pub code: String,
    /// The command to run on the new device, with the code in it.
    pub command: String,
}

/// The state of this device's own join request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JoinRequestState {
    /// Network id, once known.
    pub network: Option<String>,
    /// Network name.
    pub network_name: String,
    /// The code this device shows; the admin sees the same.
    pub check_code: String,
    /// `null` while waiting.
    pub resolution: Option<Resolution>,
}

/// `POST /api/networks/:net/invites`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InviteIn {
    /// Lifetime in seconds.
    pub expires_secs: u64,
    /// How many devices may use it; `null` is unlimited.
    pub uses: Option<u32>,
}

/// A new invite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Invite {
    /// Stable id, for revoking it.
    pub id: String,
    /// The link.
    pub link: String,
    /// The link as an SVG QR code; `null` when too long for one.
    pub qr_svg: Option<String>,
    /// When it lapses, in Unix milliseconds.
    pub expires_at_ms: u64,
    /// Uses allowed; `null` is unlimited.
    pub uses: Option<u32>,
    /// Devices that used it so far, by name.
    pub used_by: Vec<String>,
}

/// `POST /api/networks/:net/joins/:node/approve`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApproveIn {
    /// The name the device joins under; `null` or absent keeps the one it asked for.
    #[serde(default)]
    pub name: Option<String>,
}

/// An invite that can still admit devices, from `GET /api/networks/:net/invites`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OpenInvite {
    /// Its id, for revoking it: the same as [`Invite::id`] when it was made.
    pub id: String,
    /// When it lapses, in Unix milliseconds.
    pub expires_at_ms: u64,
    /// How many more devices it admits; `null` is unlimited.
    pub uses_left: Option<u32>,
    /// When it was made, in Unix milliseconds.
    pub created_at_ms: u64,
}

/// `POST /api/networks/:net/policy`: replaces the network's policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PolicyIn {
    /// The policy as TOML, as `GET` gives it.
    pub toml: String,
}

/// One relay advertised in a network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RelayView {
    /// The relay's name, unique in the network.
    pub name: String,
    /// Its `host:port`.
    pub address: String,
    /// The device running it.
    pub node: String,
}

/// `POST /api/networks/:net/relays`: advertises a relay. `node` is the device running it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RelayIn {
    /// The relay's name.
    pub name: String,
    /// Its `host:port`.
    pub address: String,
    /// The device running it; `null` is this device.
    pub node: Option<String>,
}

/// `GET` and `POST /api/server`: whether this device offers itself as an exit and a relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServerView {
    /// Act as an exit node.
    pub exit: bool,
    /// Act as a relay.
    pub relay: bool,
}

/// `POST /api/wireguard/import`: a plain WireGuard config to run as a network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportIn {
    /// The name for the imported network.
    pub name: String,
    /// The `wg-quick` config text.
    pub conf_text: String,
}

/// `POST /api/wireguard/:net/use`: routes through an imported network, or stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportUseIn {
    /// Send its traffic through it when true.
    pub active: bool,
}

/// The network a WireGuard import made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Imported {
    /// Its id.
    pub network: String,
}

/// `POST /api/networks/:net/phones`: makes a plain WireGuard device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PhoneIn {
    /// The name it asks for.
    pub name: String,
    /// Its policy tags.
    pub tags: Vec<String>,
    /// The device it connects through; absent picks the network's default gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub gateway: Option<String>,
}

/// The phone that was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PhoneCreated {
    /// Its unique name, which differs from the one asked for when that was taken.
    pub name: String,
    /// Its device id.
    pub node: String,
    /// The kith device it connects through.
    pub gateway: String,
    /// Whether an exit is available for the all-traffic config.
    pub everything_available: bool,
}

/// `POST /api/networks/:net/phones/:node/config`: reveals a phone's private config
/// (tier 3: waits for OS confirmation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PhoneConfigIn {
    /// All traffic through the exit, rather than just the network.
    pub everything: bool,
}

/// A phone's `wg-quick` config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PhoneConfig {
    /// The config text; it holds the phone's private key.
    pub config: String,
    /// The config as an SVG QR code; `null` when too long for one.
    pub qr_svg: Option<String>,
}

/// `POST /api/networks/:net/records`: sets a name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecordIn {
    /// The name, without the network suffix.
    pub name: String,
    /// The record type.
    #[serde(rename = "type")]
    #[cfg_attr(feature = "ts", ts(rename = "type"))]
    pub kind: RecordType,
    /// The data.
    pub value: String,
    /// The op this edit was based on, to detect a concurrent change.
    pub based_on: Option<String>,
}

/// `POST /api/exit`: which exit this device uses; `node: null` stops using one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExitIn {
    /// Network id of the exit; `null` with `node: null` stops.
    pub network: Option<String>,
    /// The exit device.
    pub node: Option<String>,
}

/// `POST /api/networks/:net/exits/:node`: allows a device to be an exit, or stops it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GrantIn {
    /// Allow when true, revoke when false.
    pub allowed: bool,
}

/// `POST /api/networks/:net/devices/:node/rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RenameIn {
    /// The new name.
    pub name: String,
    /// The op this edit was based on, to detect a concurrent change.
    pub based_on: Option<String>,
}

/// `POST /api/networks/:net/devices/:node/settings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceSettingsIn {
    /// The device that carries its internet traffic, or `null` for its own connection.
    pub exit: Option<String>,
    /// The device itself may not choose another exit.
    pub exit_locked: bool,
    /// Its traffic is paused.
    pub paused: bool,
}

/// `POST /api/networks/:net/devices/:node/remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RemoveIn {
    /// It was lost or stolen: also revoke its invites and names.
    pub lost: bool,
}

/// `POST /api/blocked/:id/allow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AllowIn {
    /// Return what would change without signing.
    pub preview: bool,
}

/// What allowing a blocked connection would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AllowPreview {
    /// The rule as a sentence, such as `phone may reach nas on TCP 22`.
    pub rule_sentence: String,
    /// Every path the rule opens, as sentences; one entry means nothing else changes.
    pub new_paths: Vec<String>,
}

/// Why a name is not usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum NameProblem {
    /// Empty.
    Empty,
    /// Longer than 63 characters.
    TooLong,
    /// Holds something other than a–z, 0–9 and hyphens.
    InvalidCharacter,
    /// Starts or ends with a hyphen.
    HyphenAtEdge,
    /// Another device or network already has it.
    Taken,
}

/// `GET /api/validate/name?net=&name=`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NameCheck {
    /// The name as it would be stored: lowercased, spaces as hyphens.
    pub name: String,
    /// `null` when the name is usable.
    pub problem: Option<NameProblem>,
    /// A free name close to it, when taken.
    pub suggestion: Option<String>,
}

/// A recorded network for `kith-ui --fixture`: what a daemon would answer, frozen at
/// `recorded_at_ms`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Fixture {
    /// When it was recorded; the server shifts every time by `now - recorded_at_ms`.
    pub recorded_at_ms: u64,
    /// The boot id the fixture server reports.
    pub boot: String,
    /// Status of the daemon and its networks.
    pub status: StatusView,
    /// Devices per network id.
    #[cfg_attr(feature = "ts", ts(as = "BTreeMap<String, Vec<kith_ipc::PeerInfo>>"))]
    pub peers: BTreeMap<String, Value>,
    /// Names per network id.
    pub records: BTreeMap<String, Vec<RecordView>>,
    /// Everything waiting for a person.
    pub pending: Pending,
    /// The journal, oldest first; the last entry's `seq` is the snapshot's.
    #[cfg_attr(feature = "ts", ts(as = "Vec<kith_ipc::Entry>"))]
    pub journal: Value,
    /// Traffic samples per device, oldest first.
    #[cfg_attr(feature = "ts", ts(as = "Vec<kith_ipc::TrafficSeries>"))]
    pub traffic: Value,
    /// Links as each device reports them, per network id.
    pub links: BTreeMap<String, Vec<LinkTick>>,
    /// Exits per network id.
    pub exits: BTreeMap<String, Vec<ExitView>>,
    /// Open invites per network id.
    pub invites: BTreeMap<String, Vec<OpenInvite>>,
    /// Each network's policy as TOML; a network without one allows everything.
    #[serde(default)]
    pub policies: BTreeMap<String, String>,
    /// Relays per network id.
    #[serde(default)]
    pub relays: BTreeMap<String, Vec<RelayView>>,
}
