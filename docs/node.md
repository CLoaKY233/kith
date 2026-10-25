# The node (daemon)

`kith-node` wires every plane into one daemon. This page covers what the crates alone
don't decide: how devices find each other, join, and stay in sync.

## Channels

| Channel | Carries | Transport |
|---|---|---|
| WireGuard | IP packets between members | UDP (`kith-dataplane`), or the relay |
| Control | Join, ledger sync, endpoint gossip | TCP + TLS 1.3 raw public keys, on the listen port (default 41641/tcp) |
| Relay | WireGuard datagrams and disco when there's no direct path | `kith-relay`, run by a server-mode node |
| Disco/STUN | Path probing, public address discovery | The WireGuard UDP socket |

**Authentication.** Control connections authenticate both ends by their kith identity, an Ed25519 key, using TLS raw public keys. The client pins the server's expected `NodeId`. The server accepts any key, then allows:
- members, for sync and gossip;
- invite holders, for a single `JoinRequest` only.

## Joining

1. The admin runs `kith invite`, which creates a `CreateInvite` op and a token. The token's bootstrap list holds the sponsor's reachable addresses (local, STUN-observed, port-mapped) and, if the network has one, the relay server's address.
2. The new device opens a control connection to a bootstrap address, pinned to the sponsor's `NodeId`, which the token carries. It sends `JoinRequest { proof: JoinProof }`.
3. The sponsor checks expiry against its own clock, drafts and signs the `Join` op, and inserts it. It replies `JoinAccepted`, then runs a ledger sync session on the same connection. The device ends up with the full ledger.
4. From then on, the device is a member everywhere: its NodeId is in the view, so every peer installs it as a WireGuard peer.

## Staying in sync

- **Sessions.** A node keeps a control connection to a small set of peers: the relay server if there is one, plus up to 8 peers it can reach directly. It runs a `SyncSession` with each one:
  - on connect;
  - on every local ledger change;
  - every 60 s as a safety net.
- **Gossip.** Gossip messages carry `Endpoints { node, addrs, observed_at }` for nodes the sender can reach. That's how a peer learns where to send its first disco ping.
- **On disk.** The ledger is persisted to `kith-store` after every accepted batch.

## Connecting peers

- **Candidates.** For each peer, the node feeds `kith-disco` its candidate addresses:
  - from gossip;
  - from the peer's own control connection;
  - from addresses observed in pongs.
- **Path updates.** `PathChanged` events update the peer's endpoint in the dataplane: `Direct(addr)`, or `Relay` when no direct path is trusted.
- **Hole punching.** Call-me-maybe messages travel through the relay.

## The relay server

- **Advertising.** A server-mode node (`kith server`) runs `kith-relay` and a STUN responder. An admin advertises it in the ledger as the record `relay/<name>`, whose value holds the address and the relay's NodeId.
- **Connecting.** Every member connects to every advertised relay.

## Local state

```
<state_dir>/
  identity/@device        secret, 0600
  networks/@<network-id>  ledger ops, one value per network (postcard list)
  config.toml             optional
```

## Limits of the MVP

- No LAN discovery without a bootstrap address. mDNS comes later.
- Endpoint gossip is unsigned, so a malicious member can claim false endpoints. The harm is limited: WireGuard authenticates every packet, so the worst outcome is a failed connection attempt.

## Internals

`kith-node` is a set of modules around one event loop, on a single-threaded tokio runtime. The loop owns all state. Subsystems run as tasks and talk to it only through channels, so there are no locks across `.await`.

| Module | Owns | Talks to |
|---|---|---|
| `state` | State directory: identity (created on first start, stored 0600), per-network ledger persistence | `kith-store`, `kith-identity`, `kith-ledger` |
| `network` | One `NetworkRuntime` per joined network: ledger, current `NetMap`, local operations (create network, invite, records, policy, exit) | `kith-ledger`, `kith-control`, `kith-identity` |
| `channel` | `ChannelHandler` implementation over the network runtimes; picks which peers to keep control connections to; gossips our endpoints | `kith-channel` |
| `connectivity` | Per-peer `kith-disco` path selection, STUN probing, relay clients for advertised relays, port mapping, network change handling | `kith-disco`, `kith-stun`, `kith-relay`, `kith-netmon` |
| `dns` | Zone built from all NetMaps (device names, custom records), the DNS server on the resolver address, split-DNS settings | `kith-naming`, `kith-dns` |
| `os` | Turns NetMaps into a `kith-sys` `Plan` (mesh routes, DNS, exit routing, forwarding and NAT for server mode); applies on change, reverts on shutdown | `kith-sys` |
| `ipc` | IPC server; maps each `Request` to a loop command; broadcasts `Event`s to watchers. Authorization: root or the daemon's own uid may change state; anyone may read status | `kith-ipc` |
| `server` | Server mode: relay server, STUN responder on the WireGuard port, exit-node NAT | `kith-relay`, `kith-stun` |
| `external` | Standard WireGuard peers that aren't mesh members: imported exits (wg-quick) and exported gateway clients | `kith-config` wg-quick parser, dataplane external interfaces |
| `daemon` | The event loop: wires the above to the dataplane, handles shutdown (SIGINT/SIGTERM, reverts the OS plan) | everything |

**Data flow on a ledger change.** An op arrives through sync, IPC or join. The runtime inserts it and persists it, then rebuilds its NetMap and diffs it. The diff updates three things:
- the dataplane: peers, routes, filter;
- the DNS zone;
- the OS plan.

The channel layer then calls `notify_changed` so peers sync.

**Testing without root.**
- The dataplane's `test-support` feature exposes an in-memory TUN.
- `os` takes a `kith_sys::System` implementation, so tests pass a no-op one.
- `kith-node` exposes `Node::start(NodeOptions)`, which returns a handle used by in-process multi-node tests. The real daemon (`run`) uses the same path with the real TUN and OS.
