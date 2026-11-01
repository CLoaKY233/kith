# kith-ui: plan

A web page for a kith network: who is online, add a device or a phone, names, exits, policy. Served by one or more members, opened from any device in the network.

## Shape

**A separate binary, `kith-ui`, in this workspace.** It talks to the local daemon over the IPC socket the CLI already uses (`kith-ipc`), so it needs no new daemon code to start with, and the daemon stays under its size budget. `kith ui` runs it if it is installed, the way `git foo` runs `git-foo`: that is the plugin mechanism, and it works for any future `kith-*` tool too.

Not a cargo feature of the main binary: an HTTP server and a web page would push `kith` past 5 MB and onto routers that don't want it. Not dynamic plugins (dylibs or WASM): Rust has no stable ABI, and a separate process with a typed protocol is safer and simpler. Same repo, because it shares `kith-ipc` and must move in step with it; a separate release archive, so people who don't want it never download it.

## Where it runs

Any member can run it, but it is meant for a **manager node**: an always-on device, usually the VPS or home server. It listens on that device's mesh address only (port 80), so it is reachable as `http://vps.home.kith.internal` from inside the network and never from the internet. `kith-ui --local` serves on `127.0.0.1` instead, for using it on your own laptop.

## Who may do what

No passwords. Inside the mesh, a packet's source address proves which device sent it: WireGuard only accepts it from that device's key. So the UI knows who is asking:

- **Any member** sees the dashboard: devices, online state, paths, DNS names.
- **Admins' devices** may change things: invite, add a phone, remove a device, set names, exits and policy.
- Everything else (outside the mesh, unknown address) gets nothing.

Changes are signed with the manager node's key, so the manager node must itself be an admin. Each change records which admin device asked for it, in the ledger operation, so the history shows who did what.

## First version

1. **Dashboard:** networks, devices, online or offline, direct or relayed, last seen, traffic. Live, from the same polling `kith watch` uses, pushed to the page with server-sent events.
2. **Add a device:** make an invite and show the link and its QR code.
3. **Add a phone:** `kith wg export` as a page, with both QR codes.
4. **Remove a device**, with a confirm step.
5. **DNS names:** list, add, delete.
6. **Exits:** which devices are exits, grant or revoke.

Later: the policy editor (with a preview of who can reach what), packet drop counters and `kith doctor` findings, several admins (needs the CLI and ledger work for adding admins first), and per-device pages.

## Building it

- **Server:** tokio plus a small HTTP/1.1 server. hyper is the safe choice; the workspace already hand-rolls HTTP for the WebSocket transport, but a UI needs forms, request checks and server-sent events, which hyper handles well.
- **Page:** server-rendered HTML with a little vanilla JS for live updates, no build step, no npm. Templates in Rust (maud) and the CSS and JS compiled into the binary, so it is one file to install.
- **Safety:** every change is a POST with an origin check, reads never change anything, and the UI refuses to start on a non-mesh address unless `--local` is given.
- **Tests:** the existing in-process daemon test harness runs a node; the UI's handlers get tested against it end to end, including the who-may-do-what rules.

## Steps

1. `crates/management/kith-ui`: binary, serves a static dashboard from `Status` and `Peers` on `--local`. `kith ui` finds and runs it.
2. Mesh-address listening and the identity check from the source address. Needs one IPC addition: ask the daemon "which device has this mesh address, and is it an admin?"
3. The six first-version pages.
4. Release archives get a separate `kith-ui` tarball; the installer asks whether to install it.
5. Recording the requesting admin in ledger operations (a ledger change, so its own step, with tests).

## Open questions

- Port 80 on the mesh address, or something less likely to clash, like 41680?
- Should every member be able to see the dashboard, or only admins? Default: every member, read-only.
- Is the money version (hosted relays, enterprise add-ons) built on this same UI? Probably yes, which is another reason to keep it a separate crate with a clean boundary.
