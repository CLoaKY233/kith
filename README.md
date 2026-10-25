# kith

> Your own private network. No account, no cloud, works on any network, runs on anything.

A WireGuard-based mesh network written in Rust. Early design stage.

- **Open and account-optional:** pair devices with an invite link; no central server required.
- **Real DNS:** custom records, your own domain, certificates.
- **Many networks at once**, alongside your existing VPN.
- **Works on any network:** automatic fallback to stealth transports on port 443.
- **Tiny:** built for routers and small devices.

`kith` is a working codename.

## Status

Pre-alpha. See [`docs/architecture.md`](docs/architecture.md) and [`docs/research/`](docs/research/).

## License

Dual-licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
