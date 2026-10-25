# kith architecture (MVP)

Status: **proposal, awaiting approval**. Date: 2026-10-26.
Background: [`research/tailscale-market-research.md`](research/tailscale-market-research.md).

---

## 1. Principles

1. **Sans-IO cores, thin async drivers.** Every protocol and plane core is a synchronous state machine:

   ```
   handle_*(now, input)
   poll_transmit()
   poll_timeout()
   poll_event()
   ```

   Cores do no sockets, no timers, no threads and no runtime. Drivers own the I/O. This is the quinn-proto and str0m pattern. It gives us:
   - deterministic simulation tests;
   - fuzzable code;
   - no runtime lock-in;
   - a small binary;
   - no lock-ordering bugs.
2. **One crate, one responsibility.** A flat `crates/` directory, matklad style. Dependencies point one way only:

   ```
   foundation  ←  plane cores  ←  drivers  ←  node  ←  binary
   ```

   No crate depends on a crate in a higher layer.
3. **`unsafe` lives in exactly one crate (`kith-sys`).**
   - Every other crate has `#![forbid(unsafe_code)]`.
   - Every unsafe block needs a `// SAFETY:` comment, enforced by lint.
4. **No panics in library code.**
   - `unwrap`, `panic!`, `todo!` and `unimplemented!` are denied.
   - `expect` is warned and allowed only with an invariant message.
   - Slice indexing is warned in parsers, which use `get()` and checked arithmetic instead.
5. **Zero allocations per packet on the hot path.** Use preallocated buffer pools, batch I/O (GSO/GRO, sendmmsg/recvmmsg), and an LPM-trie route lookup. Performance claims are proven by benchmarks (divan) and enforced in CI with a binary-size budget.
6. **Small and portable by default.**
   - Foundation crates are `no_std + alloc`.
   - Heavy features (QUIC, TLS, mDNS, hickory) sit behind cargo features.
   - Features are additive and checked with `cargo hack`.
7. **Secrets are typed.**
   - Secret key types `zeroize` on drop.
   - They print as `Debug` = `"<redacted>"`.
   - Comparisons are constant-time (`subtle`).
   - There is no `Clone` on private keys unless it's required.
8. **Sign bytes, keep bytes.** A signed object stores its exact encoded bytes. We never re-encode in order to verify a signature.

---

## 2. Planes

```
                        ┌──────────────────────────────┐
                        │ Management plane             │  CLI, IPC, config, store, metrics
                        └──────────────┬───────────────┘
                                       │ commands / events
┌──────────────────┐   NetworkView   ┌─▼──────────────────────────┐   NetMap    ┌──────────────────┐
│ Trust plane      │ ──────────────▶ │ Control plane              │ ──────────▶ │ Naming plane     │
│ identity, ledger │                 │ multi-network manager,     │             │ DNS zone, stub   │
│ policy, invites  │ ◀── ops ─────── │ sync, netmap derivation    │             │ resolver, mDNS   │
└──────────────────┘                 └─┬───────────────────────▲──┘             └──────────────────┘
                                       │ PeerSet / Filter      │ PathUp/Down, stats
                         ┌─────────────▼──────────┐   Link   ┌─┴─────────────────────┐
                         │ Data plane             │ ◀──────▶ │ Connectivity plane    │
                         │ TUN, WireGuard, filter,│          │ disco, STUN, portmap, │
                         │ router, exit           │          │ transport ladder,     │
                         └────────────────────────┘          │ relay                 │
                                                             └───────────────────────┘
```

| Plane | Owns | Consumes | Emits |
|---|---|---|---|
| **Trust** | device keys, signed membership ledger, invites, ACL compile, address allocation | ops from sync, local admin commands | `NetworkView` (members, roles, policy, DNS records) |
| **Control** | one runtime per joined network, ledger sync, netmap derivation, persistence | `NetworkView`, peer sync messages | `NetMap` (peers, allowed IPs, routes, DNS zone, filter) |
| **Connectivity** | endpoint gathering, disco (ping/pong/call-me-maybe), STUN, port mapping, transport ladder, relay client/server, path selection | `PeerSet` (keys + address hints) | `PathUp/PathDown(peer, Link)`, observed endpoints |
| **Data** | TUN, WireGuard sessions, packet filter, per-network routing, exit routing | `PeerConfig`, `FilterRules`, Links | `HandshakeNeeded`, per-peer stats |
| **Naming** | zone from NetMap + custom records, stub resolver, split DNS, OS resolver integration, mDNS bridge (later) | `NetMap` | answers, OS DNS config |
| **Management** | IPC server, config, state store, event bus, metrics | user commands | status, events |

**Multi-network:** the control plane runs one `NetworkRuntime` per joined network. Each has its own address range, routes, filter and DNS zone. The data plane keys every peer by `(NetworkId, NodeId)`, so networks never leak into each other.

---

## 3. Crates

`[core]` means sans-IO, `#![no_std] + alloc`, `forbid(unsafe)`. `[drv]` means it's a driver and uses tokio. Every crate is `publish = false` except the future `kith` facade.

### Foundation
| Crate | Purpose |
|---|---|
| `kith-types` `[core]` | `NodeId`, `NetworkId`, `OpId`, key newtypes, `Timestamp`/clock trait, IP-range types, shared error kinds |
| `kith-crypto` `[core]` | Narrow facade: Ed25519 sign/verify, X25519, ChaCha20-Poly1305, BLAKE3. `zeroize`, `subtle`. Nothing else touches raw crypto crates |
| `kith-wire` `[core]` | Framing (magic + type + version varint), length-prefixed codec, TLV extensions, postcard helpers, golden-vector test support |

### Trust plane
| Crate | Purpose |
|---|---|
| `kith-identity` `[core]` | Device identity, `KeyStore` trait, invite tokens (`kith://join/…`, base32, QR-friendly) |
| `kith-ledger` `[core]` | Signed causal DAG of ops, validation, remove-wins, admin threshold, checkpoints, derives `NetworkView` |
| `kith-policy` `[core]` | ACL model + parser, compiles to `FilterRules`; deterministic address allocation |

### Control plane
| Crate | Purpose |
|---|---|
| `kith-sync` `[core]` | Ledger reconciliation (range-based set reconciliation over op hashes) |
| `kith-control` `[core]` | `NetworkRuntime`: ledger + policy + sync → `NetMap`; multi-network manager |

### Connectivity plane
| Crate | Purpose |
|---|---|
| `kith-stun` `[core]` | Minimal STUN binding codec |
| `kith-disco` `[core]` | Disco messages, endpoint candidates, path probing, best-path selection |
| `kith-transport` `[core]` | `Transport` / `Link` traits, ladder scheduler (happy-eyeballs racing, per-network-fingerprint cache) |
| `kith-obfs` `[core]` | AmneziaWG-style packet transform (junk, padding, header remap), parameterized from the ledger |
| `kith-relay-proto` `[core]` | Relay frame codec and client/server state machines |
| `kith-transport-udp` `[drv]` | UDP link via quinn-udp (GSO/GRO, batching) |
| `kith-transport-quic` `[drv]` | QUIC-on-443 link (quinn + rustls), feature-gated |
| `kith-transport-ws` `[drv]` | TLS/WebSocket-on-443 link (CDN-able), feature-gated |
| `kith-relay` `[drv]` | Relay server + client drivers (any node can relay) |
| `kith-netmon` `[drv]` | Interface/route change monitoring, port mapping (netwatch, portmapper) |

### Data plane
| Crate | Purpose |
|---|---|
| `kith-packet` `[core]` | Zero-copy IPv4/IPv6/TCP/UDP/ICMP header views, checksums |
| `kith-filter` `[core]` | Stateful packet filter evaluating `FilterRules` |
| `kith-router` `[core]` | Per-network LPM trie: destination IP → `(NetworkId, NodeId)`, exit routes |
| `kith-wg` `[core]` | `Tunnel` trait over gotatun/boringtun noise (sans-IO); kernel-WG backend lives in the driver |
| `kith-tun` `[drv]` | TUN device (tun-rs) with offloads, batched read/write |
| `kith-dataplane` `[drv]` | Hot loop: TUN ⇄ filter ⇄ router ⇄ WG ⇄ Link; buffer pools |

### Naming plane
| Crate | Purpose |
|---|---|
| `kith-naming` `[core]` | Zone built from `NetMap` + custom records (A/AAAA/CNAME/SRV/TXT), answer function |
| `kith-dns` `[drv]` | Stub DNS server, upstream forwarding (DoH later), split DNS |

### Platform
| Crate | Purpose |
|---|---|
| `kith-sys` | **The only crate allowing `unsafe`.** Per-OS modules for routes, addresses, DNS config, socket options, firewall (nftables on Linux, no iptables dependency) |

### Management plane
| Crate | Purpose |
|---|---|
| `kith-config` | Config schema, TOML loading, validation |
| `kith-store` | State directory, atomic writes, ledger persistence (`Store` trait + fs impl) |
| `kith-ipc` | CLI ⇄ daemon protocol: postcard frames over a Unix socket or named pipe; peer-credential auth; JSON debug mode |
| `kith-node` `[drv]` | Composition root: wires every plane through typed channels on tokio `current_thread` |
| `kith-cli` | The single multi-call binary `kith` (`kith daemon`, `kith up`, `kith invite`, `kith join`, `kith status`, …) |

### Tooling (not shipped)
| Crate | Purpose |
|---|---|
| `kith-sim` | Deterministic multi-node simulator: virtual time, NAT models (full-cone, symmetric, CGNAT), partitions, packet loss. Drives the sans-IO cores directly |
| `xtask` | Size budget check, dist builds, release automation |
| `fuzz/` | cargo-fuzz targets for every parser (outside the workspace) |

That makes **33 workspace crates**. Later crates: `kith-transport-reality`, `kith-mdns`, `kith-coordinator` (optional OIDC server + web UI), `kith-ffi` (C/Go/Python bindings), `kith` (embeddable facade).

---

## 4. Wire protocols

**Common envelope:** `magic(2) | type(u8) | version(varint) | body`. Bodies are postcard. Unknown TLV extensions are skipped, which keeps things forward-compatible. Every format gets golden test vectors and a fuzz target.

| Protocol | Carried over | Shape |
|---|---|---|
| WireGuard | `Link` | Standard WG. Rung 0 stays plain WG so a bring-your-own exit works |
| Disco | `Link`, relay | `Ping{tx_id}`, `Pong{tx_id, observed_addr}`, `CallMeMaybe{endpoints}`, `TransportHint`; sealed with an X25519 box keyed to the node key; obfuscated the same way as WG when obfs is on |
| Ledger op | sync | `Op{parents, author, lamport, body, sig}`; `OpId = BLAKE3(bytes)`; Ed25519 signature over domain-separated bytes |
| Ledger sync | tunnel stream | Range-based set reconciliation over `OpId`s, then op transfer in causal order |
| Relay | any `Transport` | `[type u8][len u16][dst NodeId][payload]`; signed-challenge auth (members only); bandwidth caps |
| Invite | link / QR | `kith://join/<base32(net_id, genesis, bootstrap, invite_key, expiry, uses)>` |
| IPC | Unix socket / named pipe | Length-prefixed postcard request/response + `watch` event stream; version `hello` |

**Membership model:**
- **Genesis** defines the admin set and a k-of-n threshold.
- **Authorization:** an op is valid only if its author was authorized at the op's causal parents.
- **Remove-wins:** a removal invalidates concurrent ops by the removed key.
- **Policy and DNS conflicts** resolve last-writer-wins per key, ordered by `(lamport, OpId)`.
- **Checkpoints** (threshold-signed) prune history and block backdating.

---

## 5. Key dependencies

| Need | Choice | Why |
|---|---|---|
| WireGuard | **gotatun** (Mullvad, active), boringtun as fallback; kernel WG via `defguard_wireguard_rs` on Linux | Maintained Rust WG; kernel path where available |
| Crypto | **RustCrypto** (chacha20poly1305), **dalek 3** (Ed25519/X25519), **blake3** | Pure Rust, cross-compiles to MIPS, no C toolchain |
| TLS / QUIC | **rustls** (pure-Rust provider on MIPS, aws-lc-rs optional elsewhere), **quinn** | Sans-IO `quinn-proto`, no OpenSSL |
| TUN / UDP | **tun-rs** (offloads), **quinn-udp** (GSO/GRO, mmsg) | Batch I/O is where the throughput comes from |
| Linux net | **rtnetlink**, **rustables** | Pure Rust, nftables native |
| NAT | **portmapper**, **netwatch** (n0) + our own disco and STUN | iroh's magicsock is not extractable |
| DNS | **domain** (NLnet Labs) wire format + our own stub | Leaner than hickory |
| Runtime | **tokio** `current_thread`, minimal features | Ecosystem fit, low RAM |
| Encoding | **postcard** + explicit version envelope | no_std, tiny |
| CLI | **bpaf** | Much smaller than clap |
| Errors | **thiserror** in libraries, **anyhow** in `kith-cli` only | Library standard |
| Logging | **tracing** + minimal `fmt` subscriber; trace/debug compiled out in the embedded profile | Size |
| Testing | **proptest**, **insta**, **divan**, `kith-sim`, **miri** (nightly), **cargo-fuzz** | |

Banned: `openssl`, `ring` (MIPS risk), `regex` in the daemon, and git or unknown-registry sources. All enforced by `cargo-deny`.

---

## 6. Toolchain, lints, and profiles

- **Toolchain:** stable **1.99.1**, pinned in `rust-toolchain.toml`. Edition 2024, resolver 3, MSRV 1.99. Nightly is used only for miri, cargo-fuzz, `cargo-udeps`, and **MIPS builds**. MIPS has been tier 3 since Rust 1.72, so it needs `-Zbuild-std`; it runs as a separate CI job.
- **Rust lints:**
  - deny: `unsafe_code` (each crate also forbids it, except `kith-sys`), `unsafe_op_in_unsafe_fn`
  - warn: `missing_docs`, `unreachable_pub`, `unused_qualifications`
- **Clippy lints:**
  - groups: `pedantic` warn; `nursery` cherry-picked
  - deny: `unwrap_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`
  - warn: `expect_used`, `indexing_slicing`, `as_conversions`, `print_stdout`
  - `arithmetic_side_effects` is enabled per crate in the parsers (`kith-wire`, `kith-packet`, `kith-stun`, `kith-relay-proto`, `kith-obfs`)
- **CI runs `-D warnings`,** so every warning fails the build.
- **Profiles:**
  - `release`: thin LTO
  - `dist`: fat LTO, codegen-units=1, panic=abort, strip
  - `embedded`: `dist` + `opt-level="z"`
  - `profiling`: release + debug info
- **CI** (GitHub Actions):
  - fmt, clippy, typos, taplo, cargo-deny, cargo-machete
  - nextest on Linux, macOS and Windows
  - cross builds for x86_64/aarch64/armv7 musl via cargo-zigbuild, plus MIPS on nightly
  - **binary size budget gate** (< 5 MB)
  - scheduled jobs: miri, fuzz smoke tests, coverage

---

## 7. Milestones

| # | Deliverable | Done when |
|---|---|---|
| **M0** | Workspace, all crates as documented stubs, lints, CI, deny | `cargo xtask ci` is green locally and on GitHub |
| **M1** | Foundation + trust plane: types, crypto, wire, identity, ledger, policy | Ledger property tests pass for concurrent add/remove, partitions and backdating; golden vectors exist |
| **M2** | Data plane between two hosts with static peers | `kith up` with a manual peer pings across Linux and macOS; throughput benchmark exists |
| **M3** | Connectivity: STUN, disco, UDP transport, relay, hole punching | `kith-sim` passes the NAT matrix; a real relay fallback works |
| **M4** | Control: invite/join, sync, netmap, persistence, multi-network | Three devices join by link, survive restarts and work offline on a LAN |
| **M5** | Naming: stub DNS + custom records | `nas.home` and custom records resolve on all three hosts |
| **M6** | Stealth: obfs + QUIC/WS transports + ladder | Connects with UDP blocked and with a WG-fingerprinting firewall |
| **M7** | OpenWrt + budgets + benchmarks vs tailscaled | Binary < 5 MB, idle RAM < 10 MB, published comparison chart |

**Parallelism:** once M1's foundation crates are stable, each plane's core crates are independent. They can be built in parallel (one agent per plane, each in its own worktree) and integrated in `kith-node`.

## 8. Open questions

1. **Recovery when admins drop below threshold:** genesis recovery key vs social recovery.
2. **Invite use-limit overrun during a partition:** remove-wins revokes the extra joins. Is that acceptable UX?
3. **Relay abuse when every node relays:** default opt-in with bandwidth caps?
4. **Multi-network on mobile:** a single OS VPN slot means one TUN carries every network. Feasible, because routing is per network inside our router.
5. **Name:** `kith` is a codename.
