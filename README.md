# kith

[![ci](https://github.com/CLoaKY233/kith/actions/workflows/ci.yml/badge.svg)](https://github.com/CLoaKY233/kith/actions/workflows/ci.yml)

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

kith request-code home                  # or, without an invite: a kith://request/... code
kith request kith://request/...         # on the new device; waits for an admin
kith requests                           # admin: who is waiting, and from where
kith approve home laptop                # admin: let it in

kith dns set home nas A 100.64.0.2      # nas.home.kith.internal
kith dns list

kith exit allow home vps                # admin: let the device "vps" be an exit
kith exit use home vps                  # send internet traffic through it
kith exit off

kith wg export home phone --tag phone   # add a phone: scan a code in the WireGuard app
kith remove home phone

kith blocked home                       # connections the policy refused
kith allow home phone nas tcp 22        # admin: allow one of them

kith map                                # how each device reaches the others
kith watch                              # follow changes as they happen
kith down                               # off, and stays off after a restart
kith up
```

`kith --help` lists every command; `kith <command> --help` shows its options. To turn a VPS into an always-on exit and relay, see [docs/server.md](docs/server.md).

### Web UI

`kith ui` opens your networks in the browser: devices, their connections, traffic and recent events, live. Admins can also let waiting devices in, make invites, add phones, set DNS names and exits, and allow blocked connections. Showing a phone's private key or leaving a network asks the OS to confirm it is you.

Run with `kith ui`, it runs on your own device, listens on 127.0.0.1 only, and talks to the local kith service; nothing is hosted elsewhere. (The Console below is the same program serving a domain.) The link it opens works once, and the session lasts until you stop `kith-ui`. [docs/security.md](docs/security.md#the-web-page) has the details.

It is a separate binary, `kith-ui`, for macOS and for Linux on x86_64 and aarch64. The installer asks whether to add it when run in a terminal; with `curl | sh`, set `KITH_UI=1`:

```sh
curl -fsSL https://raw.githubusercontent.com/CLoaKY233/kith/main/install.sh | KITH_UI=1 sh
```

From source: `cargo build --release -p kith-ui`, and put `kith-ui` next to `kith`.

### The Console

On a Linux server with a domain, the installer can also run the kith Console: the same web UI, served over HTTPS at that domain, with people instead of devices signing in.

```sh
curl -fsSL https://raw.githubusercontent.com/CLoaKY233/kith/main/install.sh | sh -s -- --console vpn.example.com
```

It installs kith and `kith-ui`, creates a network if the server is in none, makes the server an exit and relay with the domain as its public name (kept in `/etc/kith/kith.toml`), opens kith's port and 80/443 where ufw or firewalld runs, and prints a one-time setup link. Point the domain at the server and open 80 and 443 in any cloud firewall first; the certificate comes from Let's Encrypt.

The person who opens the setup link becomes the network's owner, signs in with a passkey and is shown recovery words to write down. Owners and admins can then invite people (accounts with the role owner, admin or member), and add devices with one-time device codes. Owners and admins sign changes with their passkey.

The Console server holds real power over the network; read [what a compromised Console can do](docs/security.md#the-console) before you run one.

## How it differs

Tailscale, Headscale and NetBird have a coordination server that every device talks to: Tailscale's, or one you host. innernet also needs a server. Nebula has no control plane, but you run a certificate authority and "lighthouse" hosts.

kith has none of these. Each network is a ledger of signed operations (who is a member, the policy, DNS names, exits) that every device keeps a copy of and syncs with the others. Devices find each other with STUN and hole punching, and fall back to relays that members run. Other differences:

- **Several networks**, each with its own policy and DNS. Every network currently uses the same address range, so one device should join only one network at a time; giving each network its own range is planned.
- **Phones as members.** The stock WireGuard app joins through an admin's kith device, with its own address, name and policy.
- **DNS records** (A, AAAA, CNAME, TXT) under `<network>.kith.internal`, alongside a name for each device.
- **Stealth transports** (obfuscated WireGuard, QUIC, WebSocket) for networks that block plain WireGuard.
- **Small.** One static binary of about 5 MB that also runs on routers.

## Status

Alpha. It works day to day across a Mac, a Linux server and phones, but expect rough edges and breaking changes to the CLI, wire formats and on-disk state.

- iOS and Android use the plain WireGuard app through a kith device; there is no kith app yet, so kith does not show up in the system's VPN settings.
- `kith up` and `kith down` work on macOS and Linux. On other systems, run `kith daemon` under your service manager.
- From the CLI, a network has one admin. The Console adds people with owner and admin roles, but the Console server itself stays an admin device (see [docs/security.md](docs/security.md#the-console)).
- A device should be in one network at a time: all networks share one address range for now.
- No outside security audit. [docs/security.md](docs/security.md) covers the threat model and known gaps.

Known issues:

- A phone added with `kith wg export` sometimes has no internet until its tunnel is turned off and on once.
- Release builds cover macOS and Linux on x86-64, arm64 and armv7. MIPS routers (most OpenWrt devices) need a build from source for now.
- The macOS build is not signed by Apple. The install script works; a binary downloaded with a browser is blocked by Gatekeeper until you allow it.

## How it works

Protocol cores are sans-IO state machines tested in a deterministic multi-node simulator, with all `unsafe` code in one crate. [docs/architecture.md](docs/architecture.md) has the design and [docs/node.md](docs/node.md) the daemon.

## License

kith is free software under the [GNU Affero General Public License v3.0](LICENSE). You can use, study, change and share it. If you run a changed version as a service that others use over a network, you must offer them its source under the same license.

Companies that want to use kith without the AGPL's terms, for example in a closed product or service, can buy a commercial license: write to laysheth.dev@gmail.com.

Copyright 2026 CLoaKY233. Contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md) for the terms they come under.
