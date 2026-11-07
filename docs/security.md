# Security model

kith is alpha software. It has had no outside audit. This page says what it protects, against whom, and where the known gaps are.

## Keys

Each device makes one Ed25519 key when it first runs. Its `NodeId` is the public half. The same key, converted to X25519, is the device's WireGuard key, so a device's identity and its tunnel key cannot drift apart. The private key stays in the state directory (`/var/lib/kith` on Linux, `/Library/Application Support/kith` on macOS), readable by root only.

A phone added with `kith wg export` is different: the exporting device generates the phone's WireGuard key and prints it inside the QR code. It is not saved anywhere, but the device that ran the export had it in memory, and anyone who sees the code has it. A phone added from the web page is the same, except that `kith-ui` keeps the phone's config in memory until it stops, so the person can show the code again after confirming to the OS.

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

## Joining by request

A device without an invite can ask to join. Any member prints a `kith://request/` code with `kith request-code`; it holds the network's id and name and that member's addresses, and no secret. The new device signs a request with its own key and sends it to that member, which gossips it to the others. The request grants nothing: it only lets in a device once an admin runs `kith approve` (or approves it on the web page), which signs an ordinary add.

- **Check that it is the right device.** A request carries only the name the device asked for, and anyone with the code can ask under any name. The web page shows the device and every admin the same six-character check code, a hash of the device's key; approve only when it matches the code on the device in front of you. It is 30 bits: enough to tell two requests apart, not a password. `kith requests` shows the device's id instead, and `kith request` does not print a check code yet, so from the CLI compare the id with `kith status` on the new device.
- **Who sees a request.** Every member holds pending requests, with the requested name and, on the member that was asked, the address it came from. A request lapses 10 minutes after the requester signed it.
- **Flooding.** Each member holds at most 32 pending requests per network and one per device. Anyone with the code can fill that list with fresh keys for 10 minutes at a time, and new requests are refused until it drains. A request code is not a secret, but sharing it only with the person who needs it keeps this away.

## What members learn about each other

Every member keeps the full ledger, so it knows every device, its address, its DNS names and the policy. Members also gossip link reports every 10 seconds to the others in each network: which peers a device reaches, whether directly or through which relay, the round-trip time and the bytes per second on each link. So every member learns who talks to whom and roughly how much, and keeps the last report from each device for a minute. `kith map` shows these numbers.

Each device also records connections the policy refused (`kith blocked`): the two devices, the protocol and port, a count and the last time.

The web page shows a member only a band (idle, light, busy) for links between two other devices; admins and a device's own links get the exact rate. This is a courtesy on screen, not a boundary: the daemon holds the exact numbers, and any local user can read them with `kith map`.

## The web page

`kith ui` starts `kith-ui`, which serves the page on `127.0.0.1` only, and asks the OS to open a one-time link in the default browser.

- **Signing in.** The link carries a random 256-bit token after `#`, so it never reaches a server log. The page trades it once for a separate secret in an `HttpOnly`, `SameSite=Strict` cookie named for the port. The token works once and for 2 minutes; the session lasts until `kith-ui` stops. Every other user on the computer can connect to the port, but nothing under `/api/` answers without that cookie.
- **Other web sites.** Every request must name `127.0.0.1` or `localhost` on the right port in `Host`, which stops DNS rebinding. Every change, and the event stream, must carry the page's own `Origin`. A strict Content Security Policy, `frame-ancestors 'none'` and no-store caching for the API cover the rest.
- **What it can do.** `kith-ui` runs as the user who started it and talks to the daemon as that user, so it can change only what that user could change with `kith`. Showing a phone's private key and leaving a network also need the OS to confirm the person at the computer (Touch ID or the login password on macOS, polkit on Linux), within 2 minutes. That guards an unattended browser tab, not the user's own account.
- **What it holds.** Invite links and phone configs made through the page stay in `kith-ui`'s memory until it stops, and the page can list them again.

## The Console

`install.sh --console <domain>` runs `kith-ui` as a public Console: HTTPS at that domain (Let's Encrypt), as the system user `kith-console` in the `kith` operator group, talking to the server's own kith daemon. People sign in with passkeys. Each person has a role in the network's ledger: owner, admin or member. Sessions last 12 hours. The Console keeps its sessions and history in SQLite under `/var/lib/kith-console`.

- **Setup.** The first link, printed by the installer, works once and for 30 minutes. Whoever opens it first becomes the owner, so open it yourself, right away. The recovery words are generated on the server during setup, so the server knew them at that moment.
- **What people can do.** Owners and admins approve devices, make invites, change policy, DNS and exits, and manage people (only owners manage owners). Their changes are signed with their passkey and checked by every device against their role in the ledger. Members use the network; for now an owner or admin adds devices for them.

### What a compromised Console can do

Root on the Console server, or anyone who controls the `kith-ui` process or the `kith` operator group there, can:

- **Act as an admin of the network, without any passkey.** The server's own device created the network (or was made its admin), and an admin device signs admin operations on its own. That includes adding devices, changing the policy, granting exits and adding admin keys. Revoking the Console from the page does not take this away yet.
- **Name any person as the author of a change.** The daemon trusts the person the Console says asked for it.
- **Swap the change behind a passkey prompt.** A passkey signs a hash the page cannot show in full, so a malicious Console can ask an owner to sign something other than what the screen says, for example adding an attacker as owner.
- **See everything the network shares**: devices, addresses, names, link reports, and the history it stores, and serve a changed page to people who sign in.
- **Use the server as an exit or relay observer**, as with any exit: it sees internet traffic of devices that use it as their exit, and who talks to whom through its relay.

It cannot:

- Read traffic between two other devices: that stays WireGuard end to end, and relayed traffic is ciphertext.
- Take over devices: it cannot read their keys or run commands on them; it can only change what the network allows.
- Act when it is down: the mesh keeps working without the Console, and devices can still be managed from an admin device with `kith`.

So treat the Console server like the network's admin key: keep it patched, limit who has root on it, and if it may be compromised, rebuild the network from a device you trust.

## What an attacker can do

**On the network path** (your ISP, café Wi-Fi): see that you use kith, the server addresses you talk to, and how much traffic flows. They cannot read or change the traffic. DNS lookups go through the tunnel only while an exit is on; otherwise they go to your normal resolver for names outside the mesh.

**A relay** (yours or another member's): sees which two devices talk, when, and how much. It cannot decrypt anything. Only members can use a relay.

**An exit node** sees your internet traffic as it leaves the tunnel, exactly as a commercial VPN provider would. Only use exits you trust.

**Someone with an invite link** can join the network once per allowed use, until it expires. Treat the link like a password. A joined device can reach whatever the policy lets it reach, which by default is every other device. Revoke one with `kith invite revoke` or on the web page; still, make invites short and single-use (`--expires 1h --uses 1`), and see the gap on backdating below.

**A stolen member device** has that device's key: it can reach what the device could, and sync the ledger, so it learns the member list, addresses, DNS names and the link reports above. Remove it from any admin device with `kith remove`; every member stops talking to it as soon as the removal reaches them.

**A leaked admin key** can do everything a single admin can: add devices, remove devices, change the policy and grant exits. The ledger can require several admins to change the admin set, but `kith create` makes a network with one admin and a threshold of 1, and the CLI has no command yet to add admins. Today, a leaked admin key means full control of that network.

**A local user on a member device** can read everything the daemon knows that is not secret: the status, peers, DNS names, link reports, refused connections and pending join requests with their addresses. Only root and members of the operator group (`admin` on macOS, `kith` on Linux) can change anything.

## Known gaps

- **Invite backdating.** The sponsor of a join sets its timestamp, so a malicious member could accept an expired invite. Threshold-signed checkpoints will fix this; until then, keep invites short-lived.
- **No key rotation yet.** A device keeps its key for life. Replace a device whose key may have leaked: remove it and join it again.
- **Recovery if admins are lost.** A network made by the Console has recovery words, but there is no page yet to use them. A network made with `kith create` has none: if every admin key is gone, nobody can change it.
- **The Console is a permanent admin.** See [What a compromised Console can do](#what-a-compromised-console-can-do).
- **Phones trust their gateway.** A phone on plain WireGuard reaches the network through one kith device, and that device enforces the policy on the phone's behalf.
- **One admin per network from the CLI.** See above; the Console adds owners and admins as people, but the CLI cannot add admins yet.
- **The web link can be seen while it opens.** The OS opens the link by starting a program with it as an argument, so another user on a shared computer who lists processes at that moment could see the token and use it first. The window lasts until the browser loads the page, and the page then says the link was already used. On a shared computer, use `kith ui --no-open` and paste the link yourself.
- **Local web sessions do not expire.** With `kith ui`, a session ends only when `kith-ui` stops. Stop it (Ctrl-C) when you are done. Console sessions last 12 hours.
- **No audit.** The ledger rules have unit tests, and the protocols run in a deterministic multi-node simulator, but nobody outside the project has reviewed the code, and there are no fuzz targets yet.

## Reporting a vulnerability

Open a private advisory at https://github.com/CLoaKY233/kith/security/advisories/new. Please do not open a public issue. Expect a reply within a week.
