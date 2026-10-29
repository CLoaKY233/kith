# Roadmap

## Fixes

- **A newly scanned phone or iPad gets no internet until it reconnects.** Seen twice, not reproduced yet. To find: log a new client's first handshake, DNS and policy drops, and reproduce with a fresh export.

## Next

- **Interactive installer.** One command (`curl … | sh`, or `kith install`) for macOS and Linux: it detects the OS and architecture, downloads a release binary, installs the service (launchd, systemd, OpenWrt init), warns about clashing VPNs such as Tailscale, and walks through creating or joining a network. Needs published releases first.
- **`kith dns list [network]`**, and every command that takes a network defaulting to the only one, as `kith peers` does. Show every record, who set it and its address.
- **`kith watch` shows what happens.** Today it prints only connects, disconnects, network changes and warnings, so a quiet network shows nothing. Print its current state when it starts, then every change: peers and phones coming and going, path changes (direct or relayed), endpoint changes, exit node changes, DNS changes, invites used, and the packet drop counters as they grow.
- **Usage help.** `kith help` with examples for common tasks (add a device, add a phone, use an exit, turn kith off for work), and `kith status` hints when something needs doing.
- **`kith devices`** with a rename and a remove for any device, phones included, and a QR re-export that keeps the same address.
- **Choose the profile per phone.** Show which profile (own devices, or all traffic) a phone is using, from its routes.
