# Roadmap

## Fixes (found 2026-10-30, testing a Mac, a phone and an iPad)

1. **Phones show offline on other devices.** The gateway announces which phones are online only when that changes, so a device that starts or joins later hears nothing until the next change. Fix: send the current state to every device that connects, and repeat it now and then.
2. **The LAST SEEN column is stale for phones.** It shows when the gateway last announced the phone, not when the phone was last heard (it says "1m ago" for a phone in use). Fix: announce how long ago the phone was heard, and show online phones as heard just now.
3. **The gateway's own phones show "via vps" on the gateway.** On the gateway they are connected directly. Fix: show `direct` there.
4. **A newly scanned phone or iPad gets no internet until it reconnects.** Seen twice. To find: log the first handshake, DNS and policy drops of a new client, and reproduce with a fresh export.

## Next

- **Interactive installer.** One command (`curl … | sh`, or `kith install`) for macOS and Linux: it detects the OS and architecture, downloads a release binary, installs the service (launchd, systemd, OpenWrt init), warns about clashing VPNs such as Tailscale, and walks through creating or joining a network. Needs published releases first.
- **`kith dns list [network]`.** Show every record, who set it and its address.
- **`kith watch` shows what happens.** Today it prints only connects, disconnects, network changes and warnings, so a quiet network shows nothing. Print its current state when it starts, then every change: peers and phones coming and going, path changes (direct or relayed), endpoint changes, exit node changes, DNS changes, invites used, and the packet drop counters as they grow.
- **Usage help.** `kith help` with examples for common tasks (add a device, add a phone, use an exit, turn kith off for work), and `kith status` hints when something needs doing.
- **`kith devices`** with a rename and a remove for any device, phones included, and a QR re-export that keeps the same address.
- **Choose the profile per phone.** Show which profile (own devices, or all traffic) a phone is using, from its routes.
