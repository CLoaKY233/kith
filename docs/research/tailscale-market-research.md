# Beating Tailscale on its pain points: research and product design

Research date: 2026-10-26. Sources: three parallel web-research passes covering Tailscale docs, pricing and blog posts, GitHub issue reaction counts, Hacker News, OpenWrt and Tailscale forums, and competitor repos. Star counts and some of the figures marked ~ come from model knowledge, so check them before quoting them publicly.

---

## 1. TL;DR

- **Your pasted research is about half wrong.** Two of its four "innovation vectors" (eBPF speed, and "fails on UDP-blocked networks") no longer hold. Building a pitch on them would get torn apart on Hacker News.
- **What users actually complain about most** (ranked by GitHub reactions):
  1. DNS and naming.
  2. Running one VPN at a time, and coexisting with other VPNs.
  3. Joining multiple networks at once.
  4. Hard-NAT relay slowness.
  5. Mobile battery drain.
  6. Dependence on a closed control plane and a mandatory Big-Tech login.
  7. No stealth in censored networks.
  8. Being too heavy for routers.
- **Positioning:** "Tailscale's UX, but fully open, no account required, works on any network, and fits on a $20 router."
  - Win on: an **open, account-optional control plane**, **DNS that does what people ask for**, **many networks plus coexistence with other VPNs**, **pluggable stealth transports**, and **a tiny footprint**.
  - Do not lead with: raw speed.

---

## 2. Fact-check of the pasted research

| Claim | Verdict | Reality |
|---|---|---|
| Control server down means your mesh breaks | **Partly true** | Existing connections keep working. New logins, new devices and ACL changes fail. A client that restarts during an outage can lose its peer map. There were 2026 outages: Mar 11 (~4h), Apr 9 and 13, Jul 16. There is no fully offline or LAN-only bootstrap. |
| tailscaled uses 30–80MB; Rust could use under 5MB | **Partly true** | Tailscale's own figure is about 10–12MB baseline on constrained builds. Real routers show 20–80MB, and OOMs on 128MB devices (OpenWrt and Teltonika threads). The binary is about 23–33MB and doesn't fit 16MB-flash routers. Under 5MB of RAM in Rust is plausible but unproven, so measure it. |
| Fails on UDP-blocked firewalls, no stealth | **First half false, second half true** | The DERP relay runs over HTTPS on 443, so UDP-blocked networks still connect, just slowly. There is **no obfuscation**: the WireGuard and DERP fingerprints are easy to block. GitHub #10696, asking for AmneziaWG-style DPI evasion, has 103 reactions. |
| Userspace Go bottlenecks 10Gbps; eBPF/AF_XDP wins | **False / outdated** | With GSO/GRO offloads, Tailscale reaches **13 Gbps** with wireguard-go on an i5-12400, faster than kernel WireGuard (11.8). There's no evidence that AF_XDP wins for this use. Speed is not a weakness you can market. |
| No microcontroller or IoT support | **Mostly true officially** | There's no official support. Community ports exist: MicroLink for ESP32, and an ESPHome component. OpenWrt has a package, but it's heavy. |

---

## 3. What Tailscale has (and does well)

**Features:**
- WireGuard mesh, NAT traversal, DERP relays, and **Peer Relays** (your own nodes as relays, 2026).
- MagicDNS, split DNS, ACLs/grants, tags.
- Tailscale SSH (with session recording on Premium).
- Serve and Funnel, exit nodes, subnet routers, 4via6, app connectors.
- Taildrop, Tailnet Lock, device posture (MDM/EDR).
- Mullvad exit add-on.
- tsnet (embed in Go), Kubernetes operator, Terraform.
- Services (VIPs), workload identity federation, JIT access, flow logs, SIEM streaming.

**Open vs closed:** the clients (tailscaled, CLI, tsnet, DERP) are BSD-3. The coordination server and admin console are closed. Headscale (Go, ~43k stars) is a tolerated community reimplementation. It supports one tailnet, has no app connectors, has no official UI, and has no HA.

**Pricing (v4, 2026-04-08):**

| Plan | Price | What you get |
|---|---|---|
| Personal | Free | 6 users, unlimited user devices, 50 tagged resources, non-commercial only |
| Standard | $8/user/mo | |
| Premium | $18/user/mo | SSO/SCIM, posture, logs and JIT are gated to paid tiers |
| Enterprise | Custom | |

Pricing is **no longer a big complaint**, so don't build the pitch on "SaaS tax" alone.

**Why people love it (we must match all of these):**
- Install, log in, and it works.
- NAT traversal that beats plain WireGuard, even through double CGNAT.
- Stable IPs and names.
- SSH, Serve and Funnel as built-in extras.
- Good docs, and broad platforms: mobile, NAS, Apple TV.
- Clients survive control-plane blips.

---

## 4. Ranked pain points (the attack surface)

| # | Pain | Evidence | Who it hurts |
|---|---|---|---|
| 1 | **DNS and naming**: no custom records, no custom domain, no CNAMEs, no mDNS, no wildcard certs, no DoH; resolv.conf overwrites | #1543 custom MagicDNS records (**1,184 reactions**, the most-wanted issue by far), #4221 (504), #1013 mDNS (295), #7081 (133) | Homelab, dev, enterprise |
| 2 | **One VPN at a time / coexistence**: the mobile OS allows one VPN; Android has to be force-stopped; you can't use your own Mullvad/Proton account; split tunneling is weak | #9301 (323), #6912, #17190 | Mobile, privacy users |
| 3 | **Multiple tailnets at once** / sharing subnets | #183 (344), #1390 (146) | Consultants, MSPs, people with work and home tailnets |
| 4 | **Hard NAT means slow shared DERP**, with no "direct only" option and poor endpoint control | #1227 (most-commented issue), #11565, #1552; partly fixed by Peer Relays | CGNAT ISPs, LTE |
| 5 | **Mobile battery** and background issues; no VPN On Demand on Android | #3363, #14095, #12086 (92) | Mobile |
| 6 | **Closed control plane, VC/US-jurisdiction trust**, outages | HN threads, isdown history | Self-hosters, EU, privacy users |
| 7 | **Login only through a Big-Tech identity provider**: no email/password, no way to move a tailnet between providers | Docs, #18676 | Home users, privacy users |
| 8 | **Censorship and DPI**: WireGuard is easy to fingerprint, and there is no stealth mode | #10696 (103) | China, Iran, Russia, schools, corporate networks |
| 9 | **Linux firewall clashes**: Docker iptables, hard iptables dependency even on nftables systems | Docs workaround, Fedora #2453924, #14063 | Devs, servers |
| 10 | **Hardcoded 100.64.0.0/10** clashes with CGNAT and other VPNs; 4via6 is paid only | #1381 | Enterprise, Kubernetes |
| 11 | **Too heavy for small devices**: 23–33MB binary, 20–80MB RAM, OOMs | OpenWrt docs and forums | Routers, IoT |
| 12 | **Platform gaps**: needs admin on Windows (121), no Wake-on-LAN (185), no Funnel custom domains (228), no UDP in Funnel, auth keys capped at 90 days | Linked issues | Devs |

---

## 5. Competitors and the gap left open

| Project | Lang / license | Self-host | Notes |
|---|---|---|---|
| Headscale | Go, BSD-3 | Yes | Only a control server; depends on Tailscale's clients; one tailnet |
| NetBird | Go, BSD-3 + AGPL | Yes | **Main threat.** Full OSS stack with SSO, ACL and posture UI. High RAM on routers; many parts to self-host |
| EasyTier | **Rust**, LGPL-3 | No server needed | **Closest to your idea.** Decentralized, multi-transport (TCP/WS/WSS/QUIC). Not WireGuard; docs are mostly Chinese; small Western mindshare |
| ZeroTier | C++, BSL | Partial | Layer-2 overlay; non-OSI license; slower |
| Nebula | Go, MIT | Yes (lighthouses) | Cert-based PKI; no UI or SSO; manual setup |
| Netmaker | Go, license changes | Yes | Trust lost over the license change and reliability |
| innernet | Rust, MIT | Yes | Mostly inactive; no relay |
| Firezone / defguard | Rust data plane | Partial | Hub-and-spoke ZTNA, not a mesh |
| iroh | Rust, MIT/Apache | Library | QUIC dial-by-key with hole punching and relays. **A strong building block, not a competitor** |
| Pangolin, Cloudflare Tunnel, Twingate, Teleport | Various | Mixed | Reverse proxy or ZTNA, not a mesh |
| AmneziaWG, Hysteria2, wstunnel, Xray/REALITY | Various | n/a | Obfuscation tools; **no mesh VPN builds this in** |

**The gap:** nobody combines Tailscale-level polish with all four of these:
- a fully open, account-optional control plane;
- built-in stealth transports;
- real DNS;
- router-class footprint.

NetBird has openness, but not stealth or a small footprint. EasyTier has decentralization and transports, but not the polish, WireGuard, identity or Western community.

---

## 6. Product design: what to build

Working name: TBD. Positioning: **"Your own private network. No account, no cloud, works on any network, runs on anything."**

### Pillar A: Open and account-optional (pains 6, 7)
- One Rust binary that is a node, a coordinator **and** a relay. There is no separate server to deploy.
- **Pair by invite link or QR code.** No account is needed. A small network is a signed membership log (CRDT) replicated on every node, so any admin node can admit devices, and nothing breaks if one node is down.
- Optional coordinator mode adds OIDC/SSO, email+passkey login, and a web UI, for teams.
- **Works fully offline and air-gapped:** peers are found over mDNS on the LAN, and the network state is saved to disk and survives restarts.
- Tailnet-Lock-style security by default: every membership change is signed, so the coordinator can't add nodes.
- **`import-from-tailscale` / Headscale migration tool.** This is a key adoption lever.

### Pillar B: DNS that does what people ask for (pain 1, the #1 demand)
- Custom A, AAAA, CNAME, SRV and TXT records; your own domain (`*.home.example.com`); wildcard names.
- Automatic certificates for your own domain (ACME DNS-01) or a built-in internal CA.
- An mDNS/Bonjour bridge across the mesh, so printers, AirPlay and Home Assistant discovery work.
- DoH/DoT upstream.
- **Never overwrite resolv.conf.** Integrate cleanly with systemd-resolved, NetworkManager and Unraid instead.

### Pillar C: Many networks, plays nice with other VPNs (pains 2, 3, 10)
- Join **multiple networks at the same time**: work, home, and a client's network.
- **Configurable address ranges** (no hardcoded 100.64/10), plus automatic conflict detection.
- **Bring your own exit:** import any WireGuard config (Mullvad, Proton, your own VPS) as an exit node. On mobile, one OS VPN slot then carries both the mesh and your commercial VPN, which fixes the one-VPN-at-a-time problem.
- Per-app and per-domain split tunneling.

### Pillar D: Works on any network (pains 4, 8)
- A transport ladder that kicks in automatically:
  1. Direct WireGuard over UDP.
  2. WireGuard with AmneziaWG-style junk packets and header randomization.
  3. **QUIC/HTTP-3 on 443.**
  4. TLS/WebSocket on 443, which can go through a CDN.
  5. REALITY-style TLS camouflage.
- **Every node can be a relay by default**, and relays are trivially self-hostable, so you're never stuck on a slow shared relay.
- A "direct only" policy switch, plus control over which network interfaces are used for connections.

### Pillar E: Tiny and runs everywhere (pain 11)
- Targets: binary **under 5MB**, idle RAM **under 10MB** (stretch goal: under 5MB). Publish benchmarks against tailscaled in CI, because that is the viral chart.
- OpenWrt as a first-class platform (opkg package plus a LuCI app), MIPS/ARM builds.
- Native nftables with no hard iptables dependency, and Docker-aware rules.
- Later: a `no_std` ESP32 / embedded-Linux client.

### Pillar F: Developer extras (pains 9, 12)
- A Serve/Funnel equivalent with **custom domains and UDP**.
- SSH with the mesh's own identity, Wake-on-LAN through a peer, and file send.
- **Rootless/user mode** on Windows, macOS and Linux.
- An embeddable Rust library (like tsnet) with C, Go and Python bindings.
- Long-lived machine identity and workload identity, so auth keys don't expire after 90 days.

### Must keep from Tailscale
- Two-minute setup, and NAT traversal that just works.
- Stable IPs and names.
- Mobile apps.
- ACLs as code.
- Excellent docs.

---

## 7. What NOT to lead with

- **eBPF/AF_XDP.** Tailscale's userspace path already does 13 Gbps. Use UDP GSO/GRO, sendmmsg batching and TUN offloads instead; that's where the speed actually comes from. Keep eBPF as a later, optional Linux fast path.
- **A custom crypto protocol.** Use WireGuard (boringtun or Mullvad's Rust gotatun) or QUIC with standard TLS 1.3. Nobody trusts homemade crypto.
- **"Pricing" as the headline.** Free Tailscale is now generous (6 users).
- **Enterprise features before the core is loved.** SSO, RBAC and audit logging can be the paid tier later (open core).

---

## 8. Key architecture decision to make first

**Data plane: WireGuard or QUIC?**

| | WireGuard (boringtun/gotatun) | QUIC datagrams (quinn / iroh) |
|---|---|---|
| Interop | Can import Mullvad/Proton configs, kernel WireGuard fast path | No WireGuard interop |
| Stealth | Needs separate obfuscation layers | Looks like HTTP/3 on 443, so stealth comes nearly free |
| Hole punching and relays | Must build yourself | iroh already provides them |
| Throughput | Proven high | Userspace QUIC is CPU-heavier |

**Recommendation:** use WireGuard as the main data plane, and QUIC/TLS as fallback transports in the ladder. Consider reusing iroh's hole-punching and relay code for the connection layer. This keeps bring-your-own-exit (Pillar C) and gives stealth (Pillar D).

**Suggested stack:**
- tokio for the async runtime.
- boringtun or gotatun for WireGuard; quinn for QUIC; rustls for TLS.
- A hickory-dns based resolver.
- automerge or a custom CRDT for the membership log.
- tun-rs for the TUN interface.
- nftables via netlink.

---

## 9. Phased plan

1. **MVP (the HN launch):** Pillars A, B and D core, plus E benchmarks.
   - Linux, macOS, Windows and OpenWrt.
   - Invite-link pairing, offline LAN mode, custom DNS records, and a relay built into every node.
   - The UDP to QUIC-443 to WebSocket-443 transport ladder.
   - The memory and binary-size comparison chart.
2. **v0.2:** multiple networks at once, bring-your-own WireGuard exit, configurable address ranges, Tailscale/Headscale importer, Android app.
3. **v0.3:** iOS app, Serve/Funnel equivalent, SSH, mDNS bridge, REALITY-style transport.
4. **v1 / open core:** an OIDC coordinator with a web UI, ACL-as-code, audit logs, and a hosted option. Charge for fleet management, SSO/SCIM and compliance.

## Gaps in this research

- I didn't verify Reddit sentiment directly, because search returned little; Reddit's own search would fill this in.
- There are no measured RAM numbers for NetBird, boringtun or EasyTier. Benchmark them yourself.
- EasyTier deserves a hands-on test before committing, since it's the closest existing project.
