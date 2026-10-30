# kith

A WireGuard mesh VPN with no coordination server.

Devices join a network with an invite link and keep a signed membership ledger among themselves. There is no account to make and no server that has to stay up for the network to work: any member can let in a device that holds an invite, and the network carries on when any one machine is off.

## Install

macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/CLoaKY233/kith/main/install.sh | sh
```

It downloads the release for your system, checks it against the published SHA-256 sums, installs `/usr/local/bin/kith`, sets up the service (launchd or systemd) and turns kith on. Read the script first if you like; it is short.

From source: `cargo build --release -p kith-cli`, then `sudo target/release/kith up`.

## Use

```sh
kith create home                        # a new network, with this device as its admin
kith invite home --expires 1h --uses 1  # prints a kith://join/... link

kith join kith://join/...               # on the other device
kith status                             # who is online, and direct or relayed

kith dns set home nas A 100.64.0.2      # nas.home.kith.internal
kith dns list

kith exit allow home vps                # admin: let the device "vps" be an exit
kith exit use home vps                  # send internet traffic through it
kith exit off

kith wg export home phone --tag phone   # add a phone: scan a code in the WireGuard app
kith remove home phone

kith watch                              # follow changes as they happen
kith down                               # off, and stays off after a restart
kith up
```

`kith --help` lists every command; `kith <command> --help` shows its options. To turn a VPS into an always-on exit and relay, see [docs/server.md](docs/server.md).

## How it differs

Tailscale, Headscale and NetBird have a coordination server that every device talks to: Tailscale's, or one you host. innernet also needs a server. Nebula has no control plane, but you run a certificate authority and "lighthouse" hosts.

kith has none of these. Each network is a ledger of signed operations (who is a member, the policy, DNS names, exits) that every device keeps a copy of and syncs with the others. Devices find each other with STUN and hole punching, and fall back to relays that members run. Other differences:

- **Several networks at once**, each with its own addresses, policy and DNS, kept apart.
- **Phones as members.** The stock WireGuard app joins through an admin's kith device, with its own address, name and policy.
- **DNS records** (A, AAAA, CNAME, TXT) under `<network>.kith.internal`, alongside a name for each device.
- **Stealth transports** (obfuscated WireGuard, QUIC, WebSocket) for networks that block plain WireGuard.
- **Small.** One static binary of about 5 MB that also runs on routers.

## Status

Alpha. It works day to day across a Mac, a Linux server and phones, but expect rough edges and breaking changes to the CLI, wire formats and on-disk state.

- iOS and Android use the plain WireGuard app through a kith device; there is no kith app yet, so kith does not show up in the system's VPN settings.
- `kith up` and `kith down` work on macOS and Linux. On other systems, run `kith daemon` under your service manager.
- A network has one admin; adding more from the CLI is not done yet.
- No outside security audit. [docs/security.md](docs/security.md) covers the threat model and known gaps.

## How it works

Protocol cores are sans-IO state machines tested in a deterministic multi-node simulator, with all `unsafe` code in one crate. [docs/architecture.md](docs/architecture.md) has the design and [docs/node.md](docs/node.md) the daemon.

## License

kith is free software under the [GNU Affero General Public License v3.0](LICENSE). You can use, study, change and share it. If you run a changed version as a service that others use over a network, you must offer them its source under the same license.

Companies that want to use kith without the AGPL's terms, for example in a closed product or service, can buy a commercial license: write to laysheth.dev@gmail.com.

Copyright 2026 CLoaKY233. Contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md) for the terms they come under.
