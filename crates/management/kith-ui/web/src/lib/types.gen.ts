// Generated from kith-ipc and kith-ui-core by `cargo xtask ui-types`. Do not edit.
// Ids are lowercase hex. Times ending in `_ms` are Unix milliseconds.

/**
 * How traffic reaches a peer.
 */
export type PathKind = "direct" | "relay" | "stealth";

/**
 * Something wrong in a network, such as a phone that lost its address.
 */
export type Problem = { 
/**
 * What is wrong.
 */
what: string, 
/**
 * How to fix it.
 */
fix: string, };

/**
 * A warning the daemon logged, kept for `kith status` and `kith doctor`.
 */
export type Warning = { 
/**
 * How long ago it happened.
 */
secs_ago: number, 
/**
 * What happened.
 */
message: string, };

/**
 * One network in [`Status`].
 */
export type NetworkSummary = { 
/**
 * Network id.
 */
id: string, 
/**
 * Display name.
 */
name: string, 
/**
 * Peers online now, as [`PeerInfo::online`] decides.
 */
peers_online: number, 
/**
 * Total members.
 */
peers_total: number, 
/**
 * Relays advertised in the network.
 */
relays: number, 
/**
 * What is wrong with the network's records now, each with how to fix it.
 */
problems: Array<Problem>, 
/**
 * Whether this device is an admin of the network, so may change it.
 */
admin: boolean, 
/**
 * The settings chosen for this device in the network, if any were.
 */
my_settings: DeviceSettingsInfo | null, };

/**
 * A plain WireGuard network imported from a `wg-quick` config, in [`Status`].
 */
export type ImportedInfo = { 
/**
 * The id it is addressed by in [`Request::UseImport`] and [`Request::RemoveImport`].
 */
network: string, 
/**
 * The name it was imported under.
 */
name: string, 
/**
 * Where its server is reached, as text.
 */
endpoint: string | null, 
/**
 * What it carries, as prefixes.
 */
routes: Array<string>, 
/**
 * Whether it carries all internet traffic, so acts as an exit.
 */
everything: boolean, 
/**
 * Whether its routes go through it now.
 */
active: boolean, 
/**
 * Seconds since anything arrived from its server, if ever.
 */
last_seen_secs: number | null, 
/**
 * Bytes received from it.
 */
rx_bytes: number, 
/**
 * Bytes sent to it.
 */
tx_bytes: number, };

/**
 * The daemon's state.
 */
export type Status = { 
/**
 * This device's identity, once created.
 */
node: string | null, 
/**
 * Networks this device belongs to.
 */
networks: Array<NetworkSummary>, 
/**
 * Whether this device offers itself as an exit node.
 */
exit: boolean, 
/**
 * Whether this device offers itself as a relay.
 */
relay: boolean, 
/**
 * The exit node in use, as `network/device`.
 */
exit_node: string | null, 
/**
 * The WireGuard UDP port; the control channel uses the same TCP port.
 */
listen_port: number, 
/**
 * Where names outside the mesh are forwarded.
 */
dns_upstreams: Array<string>, 
/**
 * Packets the dataplane dropped, by reason; only reasons that occurred.
 */
drops: { [key in string]: number }, 
/**
 * Routes other interfaces hold inside kith's ranges, usually another VPN's.
 */
route_conflicts: Array<string>, 
/**
 * Recent warnings, newest last.
 */
warnings: Array<Warning>, 
/**
 * Plain WireGuard networks imported from a `wg-quick` config.
 */
imported: Array<ImportedInfo>, };

/**
 * The settings chosen for a device in a network, from `device/<node>`.
 */
export type DeviceSettingsInfo = { 
/**
 * The exit for its internet traffic; `None` is its own connection.
 */
exit: string | null, 
/**
 * Whether the device itself may not choose another exit.
 */
exit_locked: boolean, 
/**
 * Whether its traffic is paused.
 */
paused: boolean, };

/**
 * One peer in [`Response::Peers`].
 */
export type PeerInfo = { 
/**
 * Peer identity.
 */
node: string, 
/**
 * The device's name in the network: unique, and a DNS label.
 */
name: string, 
/**
 * Mesh address, as text.
 */
address: string, 
/**
 * Current path, or `None` while offline.
 */
path: PathKind | null, 
/**
 * Why that path was chosen, such as which addresses failed to answer.
 */
path_reason: string | null, 
/**
 * Seconds since anything authenticated last arrived from it, remembered across
 * restarts, or `None` if it has never been heard from.
 */
last_seen_secs: number | null, 
/**
 * The peer's current WireGuard endpoint, as text.
 */
endpoint: string | null, 
/**
 * Bytes received from the peer.
 */
rx_bytes: number, 
/**
 * Bytes sent to the peer.
 */
tx_bytes: number, 
/**
 * Whether the peer is reachable now: something authenticated arrived from it
 * recently (for a phone, at its gateway). Everything that shows liveness uses this.
 */
online: boolean, 
/**
 * The name the device asked for, when its unique `name` had to differ from it.
 */
display_name: string, 
/**
 * For a device on plain WireGuard, such as a phone, the unique name of the kith
 * device it connects through.
 */
gateway: string | null, 
/**
 * The largest packet the path to it carries, when smaller than the tunnel MTU. Bigger
 * TCP is kept smaller, IPv4 split, and senders of other traffic told to shrink.
 */
path_mtu: number | null, 
/**
 * The invite the device joined with, as its [`InviteInfo::id`]; `None` for a device
 * added another way, such as by an admin, an approved request or as a phone.
 */
invite: string | null, 
/**
 * The settings chosen for it, if any were.
 */
settings: DeviceSettingsInfo | null, 
/**
 * Whether this device is an exit and the peer sent internet traffic through it in
 * the last minute: the chosen exit is in use.
 */
via_this_exit: boolean, };

/**
 * What a journal [`Entry`] records.
 */
export type EntryKind = { "kind": "peer_online", 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * How it is reached.
 */
path: PathKind | null, 
/**
 * The kith device a plain WireGuard member goes through.
 */
gateway: string | null, } | { "kind": "peer_offline", 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, } | { "kind": "path_changed", 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * The path before.
 */
from: PathKind | null, 
/**
 * The path now.
 */
to: PathKind | null, } | { "kind": "endpoint_changed", 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * The new address.
 */
endpoint: string, } | { "kind": "device_joined", 
/**
 * The network.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * Its name.
 */
name: string, 
/**
 * Its mesh address.
 */
address: string, 
/**
 * The invite it joined with, as its [`InviteInfo::id`]; `None` for a device
 * added another way.
 */
invite: string | null, } | { "kind": "device_removed", 
/**
 * The network.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * Its name.
 */
name: string, } | { "kind": "exit_changed", 
/**
 * `network/device`.
 */
exit: string | null, } | { "kind": "dropped", 
/**
 * Why, as `kith status` names it.
 */
reason: string, 
/**
 * How many since the last entry for this reason.
 */
count: number, } | { "kind": "warning", 
/**
 * Human-readable text.
 */
message: string, } | { "kind": "log", 
/**
 * `error`, `warn`, `info`, `debug` or `trace`.
 */
level: string, 
/**
 * The module that logged it.
 */
target: string, 
/**
 * The message and its fields.
 */
message: string, } | { "kind": "join_requested", 
/**
 * The network.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * The name it asked for.
 */
name: string, } | { "kind": "blocked", 
/**
 * The network.
 */
network: string, 
/**
 * The device that tried to connect.
 */
from: string, 
/**
 * Its name in the network.
 */
from_name: string, 
/**
 * The device it tried to reach.
 */
to: string, 
/**
 * Its name in the network.
 */
to_name: string, 
/**
 * `tcp`, `udp`, `icmp`, or the IP protocol number.
 */
protocol: string, 
/**
 * The destination port, for TCP and UDP.
 */
port: number | null, 
/**
 * Packets refused since the last entry for this connection.
 */
count: number, } | { "kind": "device_renamed", 
/**
 * The network.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * Its name before.
 */
old_name: string, 
/**
 * Its name now.
 */
name: string, };

/**
 * One thing that happened, numbered so a client can resume where it stopped.
 */
export type Entry = { 
/**
 * Increases by one per entry, from 0 when the daemon started.
 */
seq: number, 
/**
 * When, in milliseconds since the Unix epoch.
 */
at_ms: number, } & ({ "kind": "peer_online", 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * How it is reached.
 */
path: PathKind | null, 
/**
 * The kith device a plain WireGuard member goes through.
 */
gateway: string | null, } | { "kind": "peer_offline", 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, } | { "kind": "path_changed", 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * The path before.
 */
from: PathKind | null, 
/**
 * The path now.
 */
to: PathKind | null, } | { "kind": "endpoint_changed", 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * The new address.
 */
endpoint: string, } | { "kind": "device_joined", 
/**
 * The network.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * Its name.
 */
name: string, 
/**
 * Its mesh address.
 */
address: string, 
/**
 * The invite it joined with, as its [`InviteInfo::id`]; `None` for a device
 * added another way.
 */
invite: string | null, } | { "kind": "device_removed", 
/**
 * The network.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * Its name.
 */
name: string, } | { "kind": "exit_changed", 
/**
 * `network/device`.
 */
exit: string | null, } | { "kind": "dropped", 
/**
 * Why, as `kith status` names it.
 */
reason: string, 
/**
 * How many since the last entry for this reason.
 */
count: number, } | { "kind": "warning", 
/**
 * Human-readable text.
 */
message: string, } | { "kind": "log", 
/**
 * `error`, `warn`, `info`, `debug` or `trace`.
 */
level: string, 
/**
 * The module that logged it.
 */
target: string, 
/**
 * The message and its fields.
 */
message: string, } | { "kind": "join_requested", 
/**
 * The network.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * The name it asked for.
 */
name: string, } | { "kind": "blocked", 
/**
 * The network.
 */
network: string, 
/**
 * The device that tried to connect.
 */
from: string, 
/**
 * Its name in the network.
 */
from_name: string, 
/**
 * The device it tried to reach.
 */
to: string, 
/**
 * Its name in the network.
 */
to_name: string, 
/**
 * `tcp`, `udp`, `icmp`, or the IP protocol number.
 */
protocol: string, 
/**
 * The destination port, for TCP and UDP.
 */
port: number | null, 
/**
 * Packets refused since the last entry for this connection.
 */
count: number, } | { "kind": "device_renamed", 
/**
 * The network.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * Its name before.
 */
old_name: string, 
/**
 * Its name now.
 */
name: string, });

/**
 * Total bytes received from and sent to a peer at one moment.
 */
export type TrafficSample = { 
/**
 * When, in milliseconds since the Unix epoch.
 */
at_ms: number, 
/**
 * Received since the session began.
 */
rx_bytes: number, 
/**
 * Sent since the session began.
 */
tx_bytes: number, };

/**
 * Traffic counters of one peer, sampled every few seconds.
 */
export type TrafficSeries = { 
/**
 * Its network.
 */
network: string, 
/**
 * The peer.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * Oldest first.
 */
samples: Array<TrafficSample>, };

/**
 * One peer in a [`LinkReport`].
 */
export type LinkInfo = { 
/**
 * The peer reached.
 */
peer: string, 
/**
 * Its name in the network, or its short id if the reporter knows a peer this device
 * does not.
 */
peer_name: string, 
/**
 * The path in use.
 */
path: PathKind, 
/**
 * The relay carrying the traffic, when relayed.
 */
relay: string | null, 
/**
 * The measured round-trip time in milliseconds, once known.
 */
rtt_ms: number | null, 
/**
 * Bytes per second both ways over the last sample.
 */
bytes_per_sec: number, };

/**
 * One device's report of the peers it reaches, from [`Response::Topology`].
 */
export type LinkReport = { 
/**
 * The reporting device.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * Seconds since the report was received; 0 for this device's own.
 */
age_secs: number, 
/**
 * The peers it reaches.
 */
links: Array<LinkInfo>, };

/**
 * A device waiting for an admin to let it join, from [`Request::PendingJoins`].
 */
export type PendingJoin = { 
/**
 * The network it asked to join.
 */
network: string, 
/**
 * The device.
 */
node: string, 
/**
 * The name it asked for.
 */
name: string, 
/**
 * Where it asked from, when it asked this device directly.
 */
from: string | null, 
/**
 * Seconds since it asked.
 */
age_secs: number, };

/**
 * A connection a network's policy refused, from [`Request::Blocked`].
 */
export type BlockedInfo = { 
/**
 * The network.
 */
network: string, 
/**
 * The device that tried to connect.
 */
from: string, 
/**
 * Its name in the network.
 */
from_name: string, 
/**
 * The device it tried to reach.
 */
to: string, 
/**
 * Its name in the network.
 */
to_name: string, 
/**
 * `tcp`, `udp`, `icmp`, or the IP protocol number.
 */
protocol: string, 
/**
 * The destination port, for TCP and UDP.
 */
port: number | null, 
/**
 * Packets refused; under a flood only a sample is counted.
 */
count: number, 
/**
 * When the latest was refused, in milliseconds since the Unix epoch.
 */
last_ms: number, 
/**
 * The policy rule, as TOML, that would allow just this connection.
 */
rule: string, };

/**
 * A person's role in a network, from [`Request::Accounts`].
 */
export type AccountRole = "owner" | "admin" | "member";

/**
 * One passkey of an [`AccountInfo`], enough to check a login assertion.
 */
export type CredentialInfo = { 
/**
 * The credential id.
 */
credential_id: Array<number>, 
/**
 * The P-256 public key, uncompressed SEC1.
 */
public_key: Array<number>, 
/**
 * The relying party id.
 */
rp_id: string, };

/**
 * One person in [`AccountsInfo`].
 */
export type AccountInfo = { 
/**
 * The account.
 */
id: string, 
/**
 * Their name.
 */
name: string, 
/**
 * Their role.
 */
role: AccountRole, 
/**
 * Their passkeys.
 */
credentials: Array<CredentialInfo>, 
/**
 * The member devices assigned to them.
 */
devices: Array<string>, };

/**
 * A network's people, from [`Request::Accounts`].
 */
export type AccountsInfo = { 
/**
 * Everyone, by name.
 */
accounts: Array<AccountInfo>, 
/**
 * The devices authorized to act for people.
 */
consoles: Array<string>, 
/**
 * Whether the network has a recovery key.
 */
recovery_set: boolean, };

/**
 * Versions of this server and of the daemon it talks to.
 */
export type Versions = { 
/**
 * `kith-ui`'s version.
 */
ui: string, 
/**
 * The daemon's version, empty while it is unreachable.
 */
daemon: string, 
/**
 * Whether the two speak the same IPC protocol.
 */
compatible: boolean, };

/**
 * This device's standing in a network.
 */
export type Role = "admin" | "member" | "view_only" | "removed";

/**
 * This device in one network: the peers list never includes it.
 */
export type Membership = { 
/**
 * Network id.
 */
network: string, 
/**
 * This device's name in the network.
 */
name: string, 
/**
 * This device's mesh address.
 */
address: string, 
/**
 * Its standing.
 */
role: Role, 
/**
 * Its key's short fingerprint, as `k7f3·9a21`.
 */
key_fpr: string, 
/**
 * The device that made it admin, when it is one and was not the founder.
 */
made_admin_by: string | null, 
/**
 * When it was made admin, in Unix milliseconds.
 */
made_admin_at_ms: number | null, 
/**
 * Every admin's name, for "Changes need an admin: desk or nas".
 */
admins: Array<string>, 
/**
 * Whether this device shares its link reports with the network.
 */
share_links: boolean, };

/**
 * The computer itself, for Welcome before any network exists.
 */
export type Host = { 
/**
 * The host name, as a suggested device name.
 */
name: string, 
/**
 * The operating system, such as `macOS` or `Linux`.
 */
os: string, };

/**
 * How the exit in use is doing.
 */
export type ExitState = "active" | "offline_blocked" | "offline_direct";

/**
 * `GET /api/status`: the daemon's `Status` plus what only the page needs.
 */
export type StatusView = { 
/**
 * Whether this OS user may change kith on this device.
 */
operator: boolean, 
/**
 * Versions of `kith-ui` and the daemon.
 */
versions: Versions, 
/**
 * When a pause ends, in Unix milliseconds; `null` when not paused.
 */
paused_until: number | null, 
/**
 * This device in each network.
 */
memberships: Array<Membership>, 
/**
 * The computer.
 */
host: Host, 
/**
 * How the exit in use is doing; `null` without one.
 */
exit_state: ExitState | null, 
/**
 * How this server is reached.
 */
mode: ConsoleMode, 
/**
 * The signed-in person; `null` for the local link's session or no session.
 */
me: Me | null, 
/**
 * This device's identity, once created.
 */
node: string | null, 
/**
 * Networks this device belongs to.
 */
networks: Array<NetworkSummary>, 
/**
 * Whether this device offers itself as an exit node.
 */
exit: boolean, 
/**
 * Whether this device offers itself as a relay.
 */
relay: boolean, 
/**
 * The exit node in use, as `network/device`.
 */
exit_node: string | null, 
/**
 * The WireGuard UDP port; the control channel uses the same TCP port.
 */
listen_port: number, 
/**
 * Where names outside the mesh are forwarded.
 */
dns_upstreams: Array<string>, 
/**
 * Packets the dataplane dropped, by reason; only reasons that occurred.
 */
drops: { [key in string]: number }, 
/**
 * Routes other interfaces hold inside kith's ranges, usually another VPN's.
 */
route_conflicts: Array<string>, 
/**
 * Recent warnings, newest last.
 */
warnings: Array<Warning>, 
/**
 * Plain WireGuard networks imported from a `wg-quick` config.
 */
imported: Array<ImportedInfo>, };

/**
 * One exit in a network, for the Exits screen and the picker.
 */
export type ExitView = { 
/**
 * Network id.
 */
network: string, 
/**
 * The exit device.
 */
node: string, 
/**
 * Its name.
 */
name: string, 
/**
 * Whether this device may use it.
 */
allowed: boolean, 
/**
 * Names of devices allowed to use it; empty when all are.
 */
access: Array<string>, 
/**
 * Names of devices using it now.
 */
in_use_by: Array<string>, };

/**
 * How busy a link is, shown to members instead of another device's exact rate.
 */
export type RateBand = "idle" | "light" | "busy";

/**
 * One device's traffic counters at a tick.
 */
export type TrafficTick = { 
/**
 * Network id.
 */
network: string, 
/**
 * Device id.
 */
node: string, 
/**
 * Bytes received from it since its session began.
 */
rx: number, 
/**
 * Bytes sent to it since its session began.
 */
tx: number, };

/**
 * One link as a device reported it, for the map, paths and latency.
 */
export type LinkTick = { 
/**
 * Network id.
 */
network: string, 
/**
 * The reporting device.
 */
from: string, 
/**
 * The device it reaches.
 */
to: string, 
/**
 * The path in use.
 */
path: PathKind, 
/**
 * The relay carrying it, when relayed.
 */
relay: string | null, 
/**
 * Round-trip time in milliseconds, once measured.
 */
rtt_ms: number | null, 
/**
 * Share of probes lost, from 0 to 1, once measured.
 */
loss: number | null, 
/**
 * Bytes per second both ways; only for this device's own links or an admin.
 */
rate: number | null, 
/**
 * How busy the link is, when `rate` is withheld.
 */
rate_band: RateBand | null, 
/**
 * Seconds since the report arrived; 0 for this device's own.
 */
age_secs: number, };

/**
 * `event: tick`, every 2 s: numbers for every device and link. Never replayed.
 */
export type Tick = { 
/**
 * When the numbers were read, in Unix milliseconds.
 */
at_ms: number, 
/**
 * Counters per device.
 */
traffic: Array<TrafficTick>, 
/**
 * Links per reporting device.
 */
links: Array<LinkTick>, };

/**
 * `event: reset`: the daemon restarted, so the page refetches everything.
 */
export type Reset = { 
/**
 * The new boot id.
 */
boot: string, 
/**
 * When the daemon started, in Unix milliseconds.
 */
started_at_ms: number, };

/**
 * `event: gap`: entries the page asked for have left the daemon's ring.
 */
export type Gap = { 
/**
 * The first missing sequence number.
 */
missed_from: number, 
/**
 * The last missing sequence number.
 */
missed_to: number, };

/**
 * What a join request says about itself; shown as a claim, never as fact.
 */
export type Claimed = { 
/**
 * The name it asked for.
 */
name: string, 
/**
 * The operating system it says it runs.
 */
os: string | null, };

/**
 * What this device saw of a join request.
 */
export type Seen = { 
/**
 * The address it asked from, when it asked this device directly.
 */
ip: string | null, 
/**
 * Its key's short fingerprint, as `k2c1·77f0`.
 */
key_fpr: string, };

/**
 * A device waiting for an admin to let it join.
 */
export type JoinRequest = { 
/**
 * Stable id, used in `?need=`.
 */
id: string, 
/**
 * Network id.
 */
network: string, 
/**
 * The requesting device's id.
 */
node: string, 
/**
 * Six Crockford base32 characters from a hash of its key; the device shows the same.
 */
check_code: string, 
/**
 * What it says about itself.
 */
claimed: Claimed, 
/**
 * What this device saw.
 */
seen: Seen, 
/**
 * When it asked, in Unix milliseconds.
 */
requested_at_ms: number, 
/**
 * When the request lapses, in Unix milliseconds.
 */
expires_at_ms: number, };

/**
 * Connections the policy refused, grouped by source, destination and port.
 */
export type BlockedGroup = { 
/**
 * Stable id, used in `?need=` and `/api/blocked/:id/allow`.
 */
id: string, 
/**
 * Network id.
 */
network: string, 
/**
 * The device that tried to connect.
 */
from: string, 
/**
 * Its name.
 */
from_name: string, 
/**
 * The device it tried to reach.
 */
to: string, 
/**
 * Its name.
 */
to_name: string, 
/**
 * `tcp`, `udp`, `icmp`, or the IP protocol number.
 */
protocol: string, 
/**
 * The destination port, for TCP and UDP.
 */
port: number | null, 
/**
 * Connections refused in the window.
 */
count: number, 
/**
 * The first refusal in the window, in Unix milliseconds.
 */
first_ms: number, 
/**
 * The latest refusal, in Unix milliseconds.
 */
last_ms: number, 
/**
 * The policy rule, as TOML, that would allow just this.
 */
rule: string, };

/**
 * `GET /api/pending`: everything waiting for a person.
 */
export type Pending = { 
/**
 * Join requests, oldest first.
 */
joins: Array<JoinRequest>, 
/**
 * Blocked connections, most recent first.
 */
blocked: Array<BlockedGroup>, };

/**
 * The snapshot inlined into `index.html` as `<script type="application/json" id="boot">`
 * and served by `GET /api/boot`.
 */
export type Boot = { 
/**
 * Identifies the daemon's run; a stream entry from another boot means refetch.
 */
boot: string, 
/**
 * The last journal sequence number this snapshot includes.
 */
seq: number, 
/**
 * The server's clock when the snapshot was taken, for the page's clock offset.
 */
server_time_ms: number, 
/**
 * Status of the daemon and its networks.
 */
status: StatusView, 
/**
 * Devices per network id.
 */
peers: { [key in string]: Array<PeerInfo> }, 
/**
 * The last 150 traffic samples per device.
 */
traffic_tail: Array<TrafficSeries>, 
/**
 * Everything waiting for a person.
 */
pending: Pending, 
/**
 * Versions of `kith-ui` and the daemon.
 */
versions: Versions, 
/**
 * The latest tick, so latency and links show on the first frame.
 */
last_tick: Tick | null, };

/**
 * The type of a name record.
 */
export type RecordType = "A" | "AAAA" | "CNAME" | "TXT";

/**
 * One name in a network.
 */
export type RecordView = { 
/**
 * The full name, such as `nas.home.kith.internal`.
 */
name: string, 
/**
 * The record type.
 */
type: RecordType, 
/**
 * The data.
 */
value: string, 
/**
 * Whether kith made it from a device's name.
 */
device: boolean, 
/**
 * The device that signed it, for names an admin set.
 */
set_by: string | null, };

/**
 * Why a request failed, as the page maps it to copy.
 */
export type ApiErrorCode = "unauthorized" | "session_used" | "session_expired" | "forbidden" | "permission_denied" | "not_found" | "conflict" | "invalid_argument" | "daemon_down" | "not_in_group" | "version_skew" | "os_auth_denied" | "rate_limited" | "internal";

/**
 * How a join request was settled.
 */
export type Resolution = "approved" | "denied" | "expired";

/**
 * Who won a race, sent with a 409.
 */
export type Conflict = { 
/**
 * The device that changed it first.
 */
resolved_by: string, 
/**
 * For a join request, what that device decided.
 */
resolution: Resolution | null, 
/**
 * When, in Unix milliseconds.
 */
at_ms: number, 
/**
 * The conflicting operation, for a change made with `based_on`.
 */
op: string | null, };

/**
 * The body of every non-2xx answer.
 */
export type ApiErrorBody = { 
/**
 * What kind of failure.
 */
code: ApiErrorCode, 
/**
 * The daemon's or server's words, for logs and the `Internal` copy.
 */
message: string, 
/**
 * Who won, for a 409.
 */
conflict: Conflict | null, };

/**
 * Where a change stands when its request returns.
 */
export type OutcomeState = "accepted" | "awaiting_os_auth";

/**
 * The answer to a change: its state and, for a ledger operation, the op id that later
 * `op_seen` entries refer to.
 */
export type Outcome = { 
/**
 * Where it stands.
 */
state: OutcomeState, 
/**
 * The ledger operation, when the change signed one.
 */
op: string | null, };

/**
 * `POST /api/session`: trades the one-time link token for a session cookie.
 */
export type SessionIn = { 
/**
 * The token from the link's `#t=` fragment.
 */
token: string, };

/**
 * `POST /api/networks`: starts a network with this device as its first admin.
 */
export type CreateNetworkIn = { 
/**
 * The network's name.
 */
name: string, 
/**
 * This device's name in it.
 */
device_name: string, };

/**
 * `POST /api/join`: joins through an invite link.
 */
export type JoinIn = { 
/**
 * The invite link.
 */
invite_link: string, 
/**
 * This device's name in the network.
 */
device_name: string, };

/**
 * `POST /api/join/request`: asks an admin to let this device in.
 */
export type RequestJoinIn = { 
/**
 * The network and admin, as `home@studio-mac`, or a `kith://request/` code.
 */
request_code: string, 
/**
 * This device's name in the network.
 */
device_name: string, };

/**
 * `GET /api/networks/:net/request-code`: what a new device runs to ask an admin to let
 * it in, for "Approve it live".
 */
export type JoinCode = { 
/**
 * The `kith://request/` code.
 */
code: string, 
/**
 * The command to run on the new device, with the code in it.
 */
command: string, };

/**
 * The state of this device's own join request.
 */
export type JoinRequestState = { 
/**
 * Network id, once known.
 */
network: string | null, 
/**
 * Network name.
 */
network_name: string, 
/**
 * The code this device shows; the admin sees the same.
 */
check_code: string, 
/**
 * `null` while waiting.
 */
resolution: Resolution | null, };

/**
 * `POST /api/networks/:net/invites`.
 */
export type InviteIn = { 
/**
 * Lifetime in seconds.
 */
expires_secs: number, 
/**
 * How many devices may use it; `null` is unlimited.
 */
uses: number | null, };

/**
 * A new invite.
 */
export type Invite = { 
/**
 * Stable id, for revoking it.
 */
id: string, 
/**
 * The link.
 */
link: string, 
/**
 * The link as an SVG QR code; `null` when too long for one.
 */
qr_svg: string | null, 
/**
 * When it lapses, in Unix milliseconds.
 */
expires_at_ms: number, 
/**
 * Uses allowed; `null` is unlimited.
 */
uses: number | null, 
/**
 * Devices that used it so far, by name.
 */
used_by: Array<string>, };

/**
 * `POST /api/networks/:net/joins/:node/approve`.
 */
export type ApproveIn = { 
/**
 * The name the device joins under; `null` or absent keeps the one it asked for.
 */
name: string | null, };

/**
 * An invite that can still admit devices, from `GET /api/networks/:net/invites`.
 */
export type OpenInvite = { 
/**
 * Its id, for revoking it: the same as [`Invite::id`] when it was made.
 */
id: string, 
/**
 * When it lapses, in Unix milliseconds.
 */
expires_at_ms: number, 
/**
 * How many more devices it admits; `null` is unlimited.
 */
uses_left: number | null, 
/**
 * When it was made, in Unix milliseconds.
 */
created_at_ms: number, };

/**
 * Which devices a policy selector names.
 */
export type SelectorInfo = { 
/**
 * `any`, `device` or `tag`.
 */
kind: string, 
/**
 * The device id or tag name; `None` for `any`.
 */
value: string | null, 
/**
 * The device's name in the network, when the selector names a known device.
 */
name: string | null, };

/**
 * One destination of a [`PolicyRuleInfo`].
 */
export type TargetInfo = { 
/**
 * The devices.
 */
selector: SelectorInfo, 
/**
 * `any`, `tcp`, `udp` or `icmp`.
 */
protocol: string, 
/**
 * The ports opened, as `start-end` ranges or single ports; empty means every port.
 */
ports: Array<string>, };

/**
 * One allow rule of a [`PolicyInfo`]: any source may reach any destination.
 */
export type PolicyRuleInfo = { 
/**
 * Which devices may send.
 */
sources: Array<SelectorInfo>, 
/**
 * What they may reach.
 */
destinations: Array<TargetInfo>, };

/**
 * A network's policy, from [`Request::GetPolicy`].
 */
export type PolicyInfo = { 
/**
 * The policy as the TOML [`Request::SetPolicy`] takes.
 */
toml: string, 
/**
 * The same rules, structured.
 */
rules: Array<PolicyRuleInfo>, };

/**
 * `POST /api/networks/:net/policy`: replaces the network's policy.
 */
export type PolicyIn = { 
/**
 * The policy as TOML, as `GET` gives it.
 */
toml: string, };

/**
 * One relay advertised in a network.
 */
export type RelayView = { 
/**
 * The relay's name, unique in the network.
 */
name: string, 
/**
 * Its `host:port`.
 */
address: string, 
/**
 * The device running it.
 */
node: string, };

/**
 * `POST /api/networks/:net/relays`: advertises a relay. `node` is the device running it.
 */
export type RelayIn = { 
/**
 * The relay's name.
 */
name: string, 
/**
 * Its `host:port`.
 */
address: string, 
/**
 * The device running it; `null` is this device.
 */
node: string | null, };

/**
 * `GET` and `POST /api/server`: whether this device offers itself as an exit and a relay.
 */
export type ServerView = { 
/**
 * Act as an exit node.
 */
exit: boolean, 
/**
 * Act as a relay.
 */
relay: boolean, };

/**
 * `POST /api/wireguard/import`: a plain WireGuard config to run as a network.
 */
export type ImportIn = { 
/**
 * The name for the imported network.
 */
name: string, 
/**
 * The `wg-quick` config text.
 */
conf_text: string, };

/**
 * The network a WireGuard import made.
 */
export type Imported = { 
/**
 * Its id.
 */
network: string, };

/**
 * `POST /api/wireguard/:net/use`: routes through an imported network, or stops.
 */
export type ImportUseIn = { 
/**
 * Send its traffic through it when true.
 */
active: boolean, };

/**
 * `POST /api/networks/:net/phones`: makes a plain WireGuard device.
 */
export type PhoneIn = { 
/**
 * The name it asks for.
 */
name: string, 
/**
 * Its policy tags.
 */
tags: Array<string>, 
/**
 * The device it connects through; absent picks the network's default gateway.
 */
gateway?: string, };

/**
 * A device that plain WireGuard devices can connect through, from [`Request::Gateways`].
 */
export type GatewayInfo = { 
/**
 * The device.
 */
node: string, 
/**
 * Its name in the network.
 */
name: string, 
/**
 * Where phones reach it, as `host:port`.
 */
endpoint: string, 
/**
 * Whether it is an exit, so phones may send all traffic through it.
 */
exit: boolean, 
/**
 * Whether it is the one picked when none is chosen.
 */
default: boolean, };

/**
 * The phone that was made.
 */
export type PhoneCreated = { 
/**
 * Its unique name, which differs from the one asked for when that was taken.
 */
name: string, 
/**
 * Its device id.
 */
node: string, 
/**
 * The kith device it connects through.
 */
gateway: string, 
/**
 * Whether an exit is available for the all-traffic config.
 */
everything_available: boolean, };

/**
 * `POST /api/networks/:net/phones/:node/config`: reveals a phone's private config
 * (tier 3: waits for OS confirmation).
 */
export type PhoneConfigIn = { 
/**
 * All traffic through the exit, rather than just the network.
 */
everything: boolean, };

/**
 * A phone's `wg-quick` config.
 */
export type PhoneConfig = { 
/**
 * The config text; it holds the phone's private key.
 */
config: string, 
/**
 * The config as an SVG QR code; `null` when too long for one.
 */
qr_svg: string | null, };

/**
 * `POST /api/networks/:net/records`: sets a name.
 */
export type RecordIn = { 
/**
 * The name, without the network suffix.
 */
name: string, 
/**
 * The record type.
 */
type: RecordType, 
/**
 * The data.
 */
value: string, 
/**
 * The op this edit was based on, to detect a concurrent change.
 */
based_on: string | null, };

/**
 * `POST /api/exit`: which exit this device uses; `node: null` stops using one.
 */
export type ExitIn = { 
/**
 * Network id of the exit; `null` with `node: null` stops.
 */
network: string | null, 
/**
 * The exit device.
 */
node: string | null, };

/**
 * `POST /api/networks/:net/exits/:node`: allows a device to be an exit, or stops it.
 */
export type GrantIn = { 
/**
 * Allow when true, revoke when false.
 */
allowed: boolean, };

/**
 * `POST /api/networks/:net/devices/:node/rename`.
 */
export type RenameIn = { 
/**
 * The new name.
 */
name: string, 
/**
 * The op this edit was based on, to detect a concurrent change.
 */
based_on: string | null, };

/**
 * `POST /api/networks/:net/devices/:node/settings`.
 */
export type DeviceSettingsIn = { 
/**
 * The device that carries its internet traffic, or `null` for its own connection.
 */
exit: string | null, 
/**
 * The device itself may not choose another exit.
 */
exit_locked: boolean, 
/**
 * Its traffic is paused.
 */
paused: boolean, };

/**
 * `POST /api/networks/:net/devices/:node/remove`.
 */
export type RemoveIn = { 
/**
 * It was lost or stolen: also revoke its invites and names.
 */
lost: boolean, };

/**
 * `POST /api/blocked/:id/allow`.
 */
export type AllowIn = { 
/**
 * Return what would change without signing.
 */
preview: boolean, };

/**
 * What allowing a blocked connection would change.
 */
export type AllowPreview = { 
/**
 * The rule as a sentence, such as `phone may reach nas on TCP 22`.
 */
rule_sentence: string, 
/**
 * Every path the rule opens, as sentences; one entry means nothing else changes.
 */
new_paths: Array<string>, };

/**
 * Why a name is not usable.
 */
export type NameProblem = "empty" | "too_long" | "invalid_character" | "hyphen_at_edge" | "taken";

/**
 * `GET /api/validate/name?net=&name=`.
 */
export type NameCheck = { 
/**
 * The name as it would be stored: lowercased, spaces as hyphens.
 */
name: string, 
/**
 * `null` when the name is usable.
 */
problem: NameProblem | null, 
/**
 * A free name close to it, when taken.
 */
suggestion: string | null, };

/**
 * A recorded network for `kith-ui --fixture`: what a daemon would answer, frozen at
 * `recorded_at_ms`.
 */
export type Fixture = { 
/**
 * When it was recorded; the server shifts every time by `now - recorded_at_ms`.
 */
recorded_at_ms: number, 
/**
 * The boot id the fixture server reports.
 */
boot: string, 
/**
 * Status of the daemon and its networks.
 */
status: StatusView, 
/**
 * Devices per network id.
 */
peers: { [key in string]: Array<PeerInfo> }, 
/**
 * Names per network id.
 */
records: { [key in string]: Array<RecordView> }, 
/**
 * Everything waiting for a person.
 */
pending: Pending, 
/**
 * The journal, oldest first; the last entry's `seq` is the snapshot's.
 */
journal: Array<Entry>, 
/**
 * Traffic samples per device, oldest first.
 */
traffic: Array<TrafficSeries>, 
/**
 * Links as each device reports them, per network id.
 */
links: { [key in string]: Array<LinkTick> }, 
/**
 * Exits per network id.
 */
exits: { [key in string]: Array<ExitView> }, 
/**
 * Open invites per network id.
 */
invites: { [key in string]: Array<OpenInvite> }, 
/**
 * Each network's policy as TOML; a network without one allows everything.
 */
policies: { [key in string]: string }, 
/**
 * Relays per network id.
 */
relays: { [key in string]: Array<RelayView> }, };

/**
 * A person's role in the network's ledger.
 */
export type PersonRole = "owner" | "admin" | "member";

/**
 * Where a person stands.
 */
export type PersonState = "active" | "invited" | "registered";

/**
 * One person, in the ledger or invited.
 */
export type PersonView = { 
/**
 * The account id (hex) once active; `p-` and hex while invited.
 */
id: string, 
/**
 * Their name.
 */
name: string, 
/**
 * Their role, or the one they will get.
 */
role: PersonRole, 
/**
 * Where they stand.
 */
state: PersonState, 
/**
 * Their devices' ids.
 */
devices: Array<string>, 
/**
 * How many passkeys they have.
 */
passkeys: number, 
/**
 * The account id of who invited them, while invited or registered.
 */
invited_by: string | null, 
/**
 * When their registration link lapses, while invited.
 */
expires_at_ms: number | null, 
/**
 * Whether this is the signed-in person.
 */
me: boolean, };

/**
 * How this server is reached.
 */
export type ConsoleMode = "local" | "public";

/**
 * The signed-in person in `GET /api/status`.
 */
export type Me = { 
/**
 * Their account id.
 */
id: string, 
/**
 * Their name.
 */
name: string, 
/**
 * Their role.
 */
role: PersonRole, };

/**
 * A network by id and name.
 */
export type NetworkRef = { 
/**
 * Its id, hex.
 */
id: string, 
/**
 * Its name.
 */
name: string, };

/**
 * `GET /api/me`: who is signed in.
 */
export type MeView = { 
/**
 * How this server is reached.
 */
mode: ConsoleMode, 
/**
 * The signed-in person; `null` for the session the local one-time link started.
 */
person: PersonView | null, 
/**
 * The network the Console manages, once there is one.
 */
network: NetworkRef | null, };

/**
 * The options to pass to `navigator.credentials.create` or `.get`, in `WebAuthn`'s own
 * JSON (`PublicKeyCredentialCreationOptionsJSON` or `...RequestOptionsJSON`).
 */
export type CeremonyOptions = { 
/**
 * For `PublicKeyCredential.parseCreationOptionsFromJSON` or
 * `parseRequestOptionsFromJSON`.
 */
public_key: unknown, };

/**
 * A browser's answer to a passkey prompt: `PublicKeyCredential.toJSON()`, unchanged.
 */
export type CredentialIn = { 
/**
 * `AuthenticationResponseJSON`.
 */
credential: unknown, };

/**
 * `GET /api/setup/<token>`: what the setup page shows.
 */
export type SetupInfo = { 
/**
 * The network this device is in; `null` when setup names a new one.
 */
network: NetworkRef | null, 
/**
 * When the link lapses.
 */
expires_at_ms: number, };

/**
 * `POST /api/setup/begin`.
 */
export type SetupBeginIn = { 
/**
 * The token from `/setup/<token>`.
 */
token: string, 
/**
 * The owner's name.
 */
name: string, 
/**
 * The new network's name; needed only when [`SetupInfo::network`] is `null`.
 */
network_name: string | null, };

/**
 * `POST /api/setup/finish` and `POST /api/register/finish`.
 */
export type TokenCredentialIn = { 
/**
 * The token from the link.
 */
token: string, 
/**
 * `RegistrationResponseJSON`: `PublicKeyCredential.toJSON()` after
 * `navigator.credentials.create`.
 */
credential: unknown, };

/**
 * The ledger change an authority prompt signs.
 */
export type AuthorityKind = "add_account" | "remove_account" | "set_role" | "authorize_console" | "revoke_console" | "add_credential";

/**
 * A passkey prompt that signs one ledger change: pass `public_key` to
 * `navigator.credentials.get`, then send the answer to `POST /api/authority/finish`.
 * Its challenge is the change's op hash.
 */
export type AuthorityChallenge = { 
/**
 * What is being signed.
 */
change: AuthorityKind, 
/**
 * `PublicKeyCredentialRequestOptionsJSON`, limited to the signer's passkeys.
 */
public_key: unknown, 
/**
 * When the prompt lapses.
 */
expires_at_ms: number, };

/**
 * `POST /api/setup/finish`: the owner, now signed in, and the prompt that authorizes
 * this device as the network's Console.
 */
export type SetupFinished = { 
/**
 * The owner.
 */
person: PersonView, 
/**
 * `authorize_console` for this device.
 */
authority: AuthorityChallenge, };

/**
 * `POST /api/setup/recovery`: the recovery words, shown once.
 */
export type RecoveryWords = { 
/**
 * The words, in order.
 */
words: Array<string>, };

/**
 * `GET /api/register/<token>`: what an invited person's page shows.
 */
export type RegisterInfo = { 
/**
 * Their name.
 */
name: string, 
/**
 * The role they will get.
 */
role: PersonRole, 
/**
 * The network.
 */
network: NetworkRef, 
/**
 * The inviter's name.
 */
invited_by: string, 
/**
 * When the link lapses.
 */
expires_at_ms: number, 
/**
 * Whether they already made their passkey.
 */
registered: boolean, };

/**
 * `POST /api/register/begin`.
 */
export type TokenIn = { 
/**
 * The token from the link.
 */
token: string, };

/**
 * `POST /api/people`: invites a person.
 */
export type PersonIn = { 
/**
 * Their name.
 */
name: string, 
/**
 * Their role; only an owner invites an owner or an admin.
 */
role: PersonRole, };

/**
 * A new person's one-time registration link.
 */
export type PersonInvite = { 
/**
 * The invited person.
 */
person: PersonView, 
/**
 * `<origin>/register/<token>`; works once.
 */
link: string, 
/**
 * When it lapses.
 */
expires_at_ms: number, 
/**
 * The link as a QR code.
 */
qr_svg: string | null, };

/**
 * `POST /api/people/<id>/role`.
 */
export type RoleIn = { 
/**
 * The new role.
 */
role: PersonRole, };

/**
 * `POST /api/authority/begin`: the change to sign.
 */
export type AuthorityChangeIn = { "kind": "add_admin", 
/**
 * The pending person's id.
 */
person: string, } | { "kind": "remove_person", 
/**
 * Their account id.
 */
person: string, } | { "kind": "set_role", 
/**
 * Their account id.
 */
person: string, 
/**
 * The new role.
 */
role: PersonRole, } | { "kind": "authorize_console", 
/**
 * The device id.
 */
device: string | null, } | { "kind": "revoke_console", 
/**
 * The device id.
 */
device: string, };

/**
 * `POST /api/authority/begin`.
 */
export type AuthorityBeginIn = { 
/**
 * The change to sign.
 */
change: AuthorityChangeIn, };

/**
 * `POST /api/device-codes`.
 */
export type DeviceCodeIn = { 
/**
 * Whose device it will be, by account id; `null` is the signed-in person. Only
 * owners and admins name someone else.
 */
person: string | null, };

/**
 * A code that lets one device join, for 10 minutes.
 */
export type DeviceCodeView = { 
/**
 * Six characters, such as `K7Q2MX`.
 */
code: string, 
/**
 * When it lapses.
 */
expires_at_ms: number, 
/**
 * `<origin>/j/<code>`.
 */
link: string, 
/**
 * `kith join <link>`.
 */
command: string, 
/**
 * The link as a QR code.
 */
qr_svg: string | null, 
/**
 * Whose device it will be.
 */
person: string, };

/**
 * `GET /j/<code>`: the invite a device joins with.
 */
export type JoinLink = { 
/**
 * A `kith://join/` link for one device, valid 10 minutes.
 */
invite_link: string, 
/**
 * The network's name.
 */
network_name: string, };

/**
 * A journal entry the Console kept.
 */
export type StoredEntry = { 
/**
 * The row's id; older pages are asked for with `before`.
 */
id: number, 
/**
 * The daemon run it belongs to.
 */
boot: string, 
/**
 * The entry, flattened as the event stream sends it.
 */
entry: Entry, };

/**
 * `GET /api/history/journal`.
 */
export type JournalPage = { 
/**
 * The entries, newest first.
 */
entries: Array<StoredEntry>, 
/**
 * `before` for the next, older page; `null` when this one reached the oldest.
 */
next_before: number | null, };

/**
 * Bytes with one device in one minute.
 */
export type TrafficMinute = { 
/**
 * The minute's start.
 */
minute_ms: number, 
/**
 * Received from the device in that minute.
 */
rx_bytes: number, 
/**
 * Sent to it.
 */
tx_bytes: number, };

/**
 * `GET /api/history/traffic`: one per device.
 */
export type TrafficHistory = { 
/**
 * The network id.
 */
network: string, 
/**
 * The device id.
 */
node: string, 
/**
 * Minutes with traffic, oldest first; quiet minutes are left out.
 */
minutes: Array<TrafficMinute>, };
