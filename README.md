# kith

Your own private network. No account, no cloud, works on any network, runs on anything.

kith is a WireGuard mesh VPN written in Rust. Devices join by invite link and keep a signed membership ledger among themselves, so there is no coordination server to sign up for, pay for or wait on.

## How it differs from Tailscale

- **No account, no control server.** Membership is a signed ledger that devices sync with each other. A network keeps working, and keeps admitting devices, when any one machine is offline.
- **Real DNS.** Custom A, AAAA, CNAME, SRV and TXT records, on a domain you choose.
- **Many networks at once.** Join several networks side by side, each with its own addresses, routes, policy and DNS, without them leaking into each other.
- **Works on hostile networks.** When plain UDP is blocked or fingerprinted, kith falls back to obfuscated WireGuard and to QUIC or TLS on port 443.
- **Small.** Built to fit routers: a single binary with a size budget under 5 MB.
- **Plain WireGuard at the edges.** Import an existing wg-quick file, or export one for a device that cannot run kith.

## Quick start

```sh
# Run the daemon (usually under your service manager)
sudo kith daemon

# Create a network and invite a second device
kith create home
kith invite home --expires 24h --uses 1     # prints a kith://join/... link

# On the other device
kith join kith://join/...

kith status
kith dns set home nas A 100.64.0.2
kith exit use home vps                      # route internet traffic through peer "vps"
kith exit off

# Turn a VPS into an always-on exit node and relay
kith server --exit --relay

# Interoperate with plain WireGuard
kith wg import office ./wg0.conf
kith wg export home phone > phone.conf
```

Run `kith --help` or `kith <command> --help` for every option.

## How it works

Each device has an Ed25519 identity. Networks are defined by a signed, causally ordered ledger of membership, policy and DNS operations, which members reconcile over an authenticated control channel. From the ledger each device derives its peer map, packet filter and DNS zone. Peers find each other with STUN and hole punching, and fall back to relays run by other members. Traffic is WireGuard.

Protocol cores are sans-IO state machines, tested in a deterministic multi-node simulator. See [docs/architecture.md](docs/architecture.md) for the design and [docs/node.md](docs/node.md) for how the daemon fits together.

## Build

```sh
cargo build --release -p kith-cli
```

The binary is `target/release/kith`.

## Status

Pre-alpha. Expect breaking changes to the CLI, wire formats and on-disk state. `kith` is a working name.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
