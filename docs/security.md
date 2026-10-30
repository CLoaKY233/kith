# Security model

kith is alpha software. It has had no outside audit. This page says what it protects, against whom, and where the known gaps are.

## Keys

Each device makes one Ed25519 key when it first runs. Its `NodeId` is the public half. The same key, converted to X25519, is the device's WireGuard key, so a device's identity and its tunnel key cannot drift apart. The private key stays in the state directory (`/var/lib/kith` on Linux, `/Library/Application Support/kith` on macOS), readable by root only.

A phone added with `kith wg export` is different: the exporting device generates the phone's WireGuard key and prints it inside the QR code. It is not saved anywhere, but the device that ran the export had it in memory, and anyone who sees the code has it.

## What runs where

| Traffic | Protection |
|---|---|
| Packets between devices | WireGuard (Noise IK, ChaCha20-Poly1305, X25519, BLAKE2s), kith's own implementation, interoperable with stock WireGuard |
| Ledger sync, joins, endpoint gossip | TLS 1.3 (rustls) with raw public keys: each side pins the other's `NodeId`, no certificate authorities |
| Path discovery (disco) | Sealed with X25519 to the receiver's key |
| Relayed traffic | Still WireGuard end to end; the relay forwards ciphertext |

The cryptography comes from RustCrypto, dalek and blake3 (pure Rust). TLS uses rustls, which uses `ring`.

## Membership: the signed ledger

There is no control server. Each network is a ledger of signed operations (add a device, remove one, set the policy, set a DNS name, grant an exit) that every member keeps a full copy of and syncs with the others. A device accepts an operation only if its signature checks and its author had the right role at that point in the history. Two devices with the same operations always compute the same members, addresses and policy.

- **Normal admin operations** (add or remove a device, policy, DNS, exits, phones) need one admin's signature.
- **Changing the admin set or the threshold** needs signatures from at least the threshold number of admins, set when the network is created.
- **Removal wins.** If an admin is removed while they concurrently sign something, that operation does not count.

## What an attacker can do

**On the network path** (your ISP, café Wi-Fi): see that you use kith, the server addresses you talk to, and how much traffic flows. They cannot read or change the traffic. DNS lookups go through the tunnel only while an exit is on; otherwise they go to your normal resolver for names outside the mesh.

**A relay** (yours or another member's): sees which two devices talk, when, and how much. It cannot decrypt anything. Only members can use a relay.

**An exit node** sees your internet traffic as it leaves the tunnel, exactly as a commercial VPN provider would. Only use exits you trust.

**Someone with an invite link** can join the network once per allowed use, until it expires. Treat the link like a password. A joined device can reach whatever the policy lets it reach, which by default is every other device. Revoke unused invites; see the gap on backdating below.

**A stolen member device** has that device's key: it can reach what the device could, and sync the ledger, so it learns the member list, addresses and DNS names. Remove it from any admin device with `kith remove`; every member stops talking to it as soon as the removal reaches them.

**A leaked admin key** can do everything a single admin can: add devices, remove devices, change the policy and grant exits. The ledger can require several admins to change the admin set, but `kith create` makes a network with one admin and a threshold of 1, and the CLI has no command yet to add admins. Today, a leaked admin key means full control of that network.

**A local user on a member device** can read the status. Only root and members of the operator group (`admin` on macOS, `kith` on Linux) can change anything.

## Known gaps

- **Invite backdating.** The sponsor of a join sets its timestamp, so a malicious member could accept an expired invite. Threshold-signed checkpoints will fix this; until then, revoke invites you no longer need.
- **No key rotation yet.** A device keeps its key for life. Replace a device whose key may have leaked: remove it and join it again.
- **No recovery if admins are lost.** If every admin key is gone, nobody can change the network.
- **Phones trust their gateway.** A phone on plain WireGuard reaches the network through one kith device, and that device enforces the policy on the phone's behalf.
- **One admin per network.** See above; adding admins from the CLI is planned.
- **No audit.** The ledger rules have unit tests, and the protocols run in a deterministic multi-node simulator, but nobody outside the project has reviewed the code, and there are no fuzz targets yet.

## Reporting a vulnerability

Open a private advisory at https://github.com/CLoaKY233/kith/security/advisories/new. Please do not open a public issue. Expect a reply within a week.
