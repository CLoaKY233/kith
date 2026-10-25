# Running a server

A server is an ordinary kith node on a host with a public address, such as an AWS EC2 instance, that also runs server mode: a relay, a STUN responder on the relay port, optional QUIC and WebSocket listeners, and exit NAT.

## Install

Build on the VPS (or cross-build and copy the binary):

```sh
cargo build --release -p kith-cli
sudo install target/release/kith /usr/local/bin/kith
```

Exit NAT is implemented for Linux only: the daemon enables `net.ipv4.ip_forward` and `net.ipv6.conf.all.forwarding` and loads an `nft` table `inet kith` that masquerades member traffic out of the default-route interface. Install `nftables` and run the daemon as root.

## Config

Write `/etc/kith/kith.toml`; every key is documented in [kith.example.toml](../crates/management/kith-config/kith.example.toml).

```toml
[daemon]
state_dir = "/var/lib/kith"
socket_path = "/run/kith/kith.sock"

[network]
listen_port = 41641

[server]
enabled = true
exit = true
relay = true
public_hostname = "vps.example.com"
quic_listen = "0.0.0.0:443"
ws_listen = "0.0.0.0:443"
ws_path = "/a8f3k2p9q1"
tls_cert = "/etc/kith/cert.pem"
tls_key = "/etc/kith/key.pem"
```

What each part does:

- `listen_port` is the WireGuard UDP port. The control channel listens on TCP on the same port.
- `relay = true` starts the relay on TCP `listen_port + 1` (41642 by default) and a STUN responder on UDP on that same port. The relay serves only members of this node's networks.
- `quic_listen` starts a QUIC listener (UDP). It accepts links only from members. Clients dial the port of their own `quic_listen`, or 443 when unset, and use `[stealth] quic_sni`, else `public_hostname`, as the SNI.
- `ws_listen` starts a WebSocket listener (TCP) that carries relay connections. It needs `relay = true`; without it the daemon logs a warning and does not listen. Only `ws_path` (default `/kith`) is upgraded.
- `tls_cert` and `tls_key` are PEM files; both must be set for the WebSocket listener to serve TLS. Omit both when a CDN or reverse proxy terminates TLS and forwards plain HTTP to `ws_listen`; then bind `ws_listen` to a port the proxy reaches, such as `127.0.0.1:8080`.

Run it under a service manager:

```sh
sudo kith daemon --config /etc/kith/kith.toml --foreground
```

## Firewall

Open these in the security group (and any host firewall):

| Port | Protocol | Purpose |
|------|----------|---------|
| 41641 | UDP | WireGuard |
| 41641 | TCP | control channel |
| 41642 | TCP | relay |
| 41642 | UDP | STUN |
| 443 | UDP | QUIC (`quic_listen`) |
| 443 | TCP | WebSocket (`ws_listen`) |

If you change `listen_port`, the relay and STUN move to `listen_port + 1`.

## Join the network

On a device already in the network, print an invite and join from the VPS:

```sh
kith invite home
sudo kith join --name vps1 '<link>'
```

## Switching roles at runtime

```sh
sudo kith server --exit --relay
```

This restarts server mode with only the relay (and its STUN responder) on `listen_port + 1` and the exit flag; it does not start the QUIC or WebSocket listeners and is not written to the config file. `kith server` with neither flag stops server mode. To keep the roles across restarts, set them in `[server]`. `kith status` shows whether the node is serving as exit and relay.

## Advertising the relay

Members pick up relays from `relay/<name>` records in the network ledger, each holding a `host:port` and the relay's node id. The CLI has no command that writes such a record yet, so a relay that is running is not used by other members until one is published.

## Using it as an exit

On the laptop:

```sh
kith exit use home vps1
kith exit off
```

`kith exit use` is refused unless the node holds an `exit/<node hex>` grant in that network's ledger, and the CLI has no command that writes this grant yet. The same choice can be made at startup with `[exit] use_exit = { network = "home", node = "vps1" }`; set `allow_lan = true` to keep the local LAN reachable.
