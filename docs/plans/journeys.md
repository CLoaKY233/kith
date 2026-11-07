# kith journeys: who it is for, what each device is, and what breaks today

Checked against the code at `1006729` (feat/kith-ui), 2026-11-07. This page covers the whole product: the daemon, the CLI, the Console and local UI, accounts and security, install and upgrade, and the docs. Every row's status was checked by reading the code and its tests. Where a run settled a question, the row says so. Nothing here was tried on real Ubuntu, Debian or macOS installs, so treat the "untested" rows as things to try first.

Status: **works** (code and a test agree), **untested** (the code looks right, but no test covers it), **broken** (the code does the wrong thing), **missing** (not built). Severity: **blocker** (fix before launch), **important** (fix soon after; users will hit it), **later**.

Path prefixes: `node/` = `crates/management/kith-node/src`, `ui/` = `crates/management/kith-ui-core/src`, `web/` = `crates/management/kith-ui/web/src`, `cli/` = `crates/management/kith-cli/src`, `ipc/` = `crates/management/kith-ipc/src`, `ledger/` = `crates/trust/kith-ledger/src`, `sys/` = `crates/platform/kith-sys/src`.

## 1. Who it is for

**The ideal user** is a technical person who looks after the devices of a small group: a homelabber, a freelancer, a founder with a team of 2 to 20, or a family's "IT person". They have 3 to 50 devices, among them a cheap VPS, and one or two phones per person. They know Tailscale, and maybe used it. They want the same ease, with three differences:

1. **No vendor in the middle.** No account with a company, no coordination server that has to stay up, no per-seat price. The network belongs to them and keeps working if their VPS dies.
2. **Their own domain and their own server.** One command on a VPS gives them an always-on exit, a relay, and a Console at `vpn.example.com` that they sign in to with a passkey.
3. **People, not keys.** Each device has an owner. Anyone can see whose phone made a request. Removing a person removes their devices.

**What they want, in order:**

| Need | What "done" looks like |
|---|---|
| Install | One line on each machine. Re-running it upgrades and restarts the service. |
| Add a computer | One command with a short code: `kith join https://vpn.example.com/j/ABC123`. It works the first time, from anywhere, behind any NAT. |
| Add a phone or Windows PC | Scan a QR in the stock WireGuard app. By default it reaches the internet through the server, and it has a name, an owner and an address. |
| Reach things by name | `nas.home.kith.internal` on every device. Phones too. |
| Exit node | Pick the server, and all traffic goes through it. If the exit dies, say so loudly and offer to turn it off. |
| See who is who | A list of people, their devices, online or not, and where each connection came from. |
| Remove | Remove a device or a person in one click. It is cut off everywhere within seconds, even when offline, and it learns that it was removed. |
| Never lose control | Losing the server, a laptop or a passkey never locks the owner out. The recovery words actually work. |
| Coexist | Runs next to Tailscale or a work VPN without a clash, and turns off cleanly for work. |
| Stay out of the way | Survives sleep, Wi-Fi to mobile, reboots, upgrades and mixed versions without manual steps. |

**Must-have, the reasons to switch:** the network survives the server; it is self-hosted with no control plane to run; phones need no app; passkey people with real ownership; one-line server setup with exit, relay and Console; it is free and open.

**Bloat, to hide or cut until the basics are solid:** stealth transports in the UI (keep them config-only); 30-day traffic history; the link-rate bands; importing outside WireGuard configs into the mesh view; custom DNS record types beyond A/AAAA/CNAME; several networks per device (keep it working, but don't feature it); threshold-signed admin sets in the UI; keyboard shortcuts and the command palette before the plain buttons are all right.

**Where Tailscale is the bar** (where we lose today): joins that never fail, MagicDNS, sleep and roaming, coexistence, silent upgrades, a phone app. We match the first five, and replace the sixth with plain WireGuard plus a gateway that does the work.

## 2. Device kinds, identities and who signs what

| Kind | Identity | Admitted by (op, signer) | Address | Name | Owner | Policy enforced by | Default route |
|---|---|---|---|---|---|---|---|
| **kith member** (Mac, Linux) | An Ed25519 device key, its `NodeId`; its WireGuard key is the same key in X25519 | `Join` (signed by the sponsor, with the invite's proof) or `AddDevice` (signed by an admin device, or the Console `Acted` for a person) | From the network range, kept for good | Asked at join; changed with `RenameDevice` | `AssignDevice` (an admin, or the Console for a person) | The receiving device (`kith-filter`; outbound is always allowed) | Mesh only; an exit if chosen |
| **Console server** | A kith member like any other. **Today** it is also the genesis admin device and an authorized console (`AuthorizeConsole`) | It founded the network (`Genesis`) | as above | as above | should be the owner's | as above | Serves as exit and relay |
| **WireGuard-only** (phones; Windows until there is a native build) | No signing key. Its id is `client_id` = BLAKE3 of its WireGuard public key (`ledger/op.rs:234`); it can never sign anything | `AddClient {key, gateway, name, tags, v4, v6}`, signed by an admin device or `Acted` by the Console | Fixed v4 /32 and v6 /128, written into its config | At creation; `RenameDevice` | `AssignDevice` | The gateway for traffic to the phone; the destination for traffic from it. Others see the phone's own address, not the gateway's | **Should be:** through the Console server as gateway and exit. **Today:** mesh only, through whichever device made the config |
| **Router running kith** (OpenWrt) | A kith member | as a member | as a member | as a member | as a member | as a member | Advertises a LAN with a `route/<prefix>` record |
| **Router on plain WireGuard** | as WireGuard-only | `AddClient` | as WireGuard-only | | | | **Missing:** `AddClient` has no extra allowed IPs, so it cannot advertise a LAN |
| **Person** | `AccountId` = BLAKE3 of their first passkey. Passkeys are P-256, rp id = the Console's domain | `AddAccount`, approved by an owner, an admin (for roles below owner) or the recovery key | — | Set at invite | — | — | — |
| **Recovery key** | Ed25519, shown once as 18 words; the ledger keeps only the public half | `SetRecoveryKey`, by the admin devices, before any person exists | | | | | |

**Who signs what (today → target):**

| Op | Today | Target |
|---|---|---|
| `Genesis` | the creating device | same |
| `AddAdmin`, `RemoveAdmin`, `SetThreshold` | admin devices, at the threshold (1 by default) | Owners (passkeys) may also `RemoveAdmin` a device, so the Console's founding-admin power can be taken back |
| Everyday admin ops (`AddDevice`, `RemoveDevice`, `CreateInvite`, `RevokeInvite`, `SetRecord`, `AddClient`, `RenameDevice`, `AssignDevice`) | Any admin device with no actor, **or** an admin device or authorized console as `Acted {actor}` | After setup, the server is an authorized console only. Every change names a person. A device may remove itself (leave = signed removal) |
| `AddAccount`, `RemoveAccount`, `SetRole`, `AuthorizeConsole`, `RevokeConsole` | `Approved` by a passkey (or the recovery key for adding people and authorizing consoles). The first owner is plain, signed by the admin devices | same, plus `AddCredential` (a second passkey) and a recovery flow that works |
| `AdmissionRequest` (ask to join) | the new device | same |

**Every member should be able to tell who a request came from.** The building blocks exist: source addresses are not rewritten inside the mesh, WireGuard allowed-IPs prevent spoofing, `kith blocked` and the journal map an address to a device (`node/blocked.rs:231-237`). What is missing is the person: `PeerInfo` and `BlockedInfo` carry no owner (`ipc/message.rs:772, 914`), there are no person selectors in the policy (`kith-policy/src/policy.rs:81-89`), and there is no log of allowed connections.

## 3. Journeys, step by step, with today's status

Each journey lists the ideal steps, then every edge case with what the code does now.

### J1. Install (laptop, server, re-run, upgrade, uninstall)

Ideal: `curl … | sh` downloads and verifies the binary, sets up the service and the operator group, turns kith on, and says what to do next. Re-running it upgrades in place and restarts what changed. `--console vpn.example.com` also creates the network, sets `public_hostname`, opens the right ports, installs the Console and prints the setup link.

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J1.1 | Fresh Ubuntu 22.04/24.04 | Installs the binary, group, unit, `kith up`. Does not check for `nft` (needed for exit NAT) or `/dev/net/tun` | untested | important | `install.sh:150-197`, `sys/linux.rs:97` |
| J1.2 | Fresh Debian 12 (no systemd-resolved, no resolvconf) | DNS falls back to rewriting `/etc/resolv.conf`, but `kith.service` has `ProtectSystem=strict` and no `ReadWritePaths`, so the write fails. DNS is applied before routes and NAT, so the whole plan fails, exit included | broken | blocker | `packaging/systemd/kith.service:20`, `sys/linux.rs:68-74, 246-269` |
| J1.3 | Fresh macOS (arm64, no `/usr/local/bin`) | Creates the directory with sudo; launchd plist; operator group `admin`. curl adds no quarantine flag | untested | — | `install.sh:141-145`, `cli/service.rs:33-45` |
| J1.4 | Tailscale running at install | Prints a note only. kith then fails to start: the same UDP port 41641 (see J17) | broken | blocker | `install.sh:186-188` |
| J1.5 | Re-run, same or newer version | Swaps the binary, but `kith up` only does `enable --now` (or nothing if launchd already loaded it), so **the old daemon keeps running**. IPC needs an exact version match, so every new CLI command fails, and `--console` ends with "kith did not start" | broken | blocker | `cli/service.rs:34,45`, `ipc/codec.rs:31`, `install.sh:203-206` |
| J1.6 | Re-run without `KITH_UI=1` on a machine with kith-ui | kith-ui is not upgraded and then fails the version check | broken | important | `install.sh:156-170` |
| J1.7 | Service files from an older release | Never rewritten once they exist | broken | important | `cli/service.rs:150` |
| J1.8 | `--console` with a domain that does not point here yet | The setup link prints. ACME keeps retrying silently, with no log line and no hint | broken | important | `ui/public.rs` (`while let Some(_event)`), `kith-ui/src/main.rs:142-144` |
| J1.9 | `--console` with port 80 or 443 taken (nginx) | The Console crash-loops, then stops at systemd's start limit. Each restart mints a new setup link, so the printed one is already dead while the installer reports success | broken | important | `kith-ui/src/main.rs:176`, `ui/store/sqlite.rs:373` |
| J1.10 | `--console` sets `server.public_hostname` | Not set. The relay gets advertised at an IPv6 address, which IPv4-only devices can't reach (test finding #1) | broken | blocker | `install.sh`, `node/daemon.rs:1089-1101` |
| J1.11 | `--console` makes the server an exit and relay that survive a reboot | `kith server` roles are runtime only. Nothing writes `[server]` | missing | blocker | `node/daemon.rs:1182-1199`, `docs/server.md:84` |
| J1.12 | `--console` on a server that is a member, not an admin, of a network | Skips create, picks that network, setup burns its link and then fails | broken | important | `install.sh:208`, `ui/console.rs:258`, `ui/api/auth.rs:209-213` |
| J1.13 | Firewall | Opens nothing. The message says 80 and 443; 41641/udp+tcp are also needed | missing | important | `install.sh:257-260` |
| J1.14 | Release has no kith-ui yet (v0.1.1) | `KITH_UI=1` and `--console` fail with "SHA256SUMS has no entry". Ship a release first | broken | blocker | `.github/workflows/release.yml` |
| J1.15 | Uninstall | No script and no docs. Leaves binaries, units, `/etc/kith`, state, the `kith` group, the `kith-console` user, and on macOS the plist, `/var/log/kith.log`, resolver files | missing | important | — |
| J1.16 | Signed binaries | SHA256SUMS sits in the same release; nothing is signed or notarized | missing | later | `release.yml` |

### J2. Console setup

Ideal: open the link, name the network and yourself, make a passkey, confirm once, write down the recovery words (the page will not go on until you confirm you saved them), land on an empty network with "Add a device" first.

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J2.1 | Happy path | Burns the token, creates the network if needed, sets the recovery key, adds the owner (signed by the device), authorizes the Console with the owner's passkey, shows the words | works | — | `ui/api/auth.rs:192-266`; `console_tests.rs::setup_runs_once_and_shows_the_recovery_words_once` |
| J2.2 | Setting the recovery key fails | The error is swallowed with `.ok()`. Setup finishes with **no recovery key**, and the page silently skips the words | broken | blocker | `ui/api/auth.rs:221`, `web/lib/setup.ts:78` |
| J2.3 | Recovery key published, then `add_account` fails | The words are lost, and the ledger holds a recovery key nobody knows. `SetRecoveryKey` can't be done again | broken | blocker | `ledger/validity.rs:463, 605-608` |
| J2.4 | Page closed, or the confirm prompt cancelled or lapsed (5 min) before the words show | The owner exists. The words stay in memory only, and nothing but Setup calls `/api/setup/recovery`. A restart loses them for good. No UI can restart `authorize_console` | broken | blocker | `web/routes/Setup.svelte:136-145`, `ui/api/auth.rs` |
| J2.5 | Any failure after the token is marked used | The link is burned; you need a restart of the Console for a new one | broken | important | `ui/api/auth.rs:209-213` |
| J2.6 | Link expired (30 min) | A restart mints a new one while setup isn't done | works | — | `ui/lib.rs:141`; `a_setup_link_lapses_after_thirty_minutes` |
| J2.7 | Console's OS user not an operator | The daemon refuses with `PermissionDenied`. The page says "Only admins can do this. This device is a member." (finding #3). `StatusView.operator` is hard-coded `true`, so the "not an operator" copy never shows. There is no startup check | broken | important | `ui/api/status.rs:330`, `web/lib/copy.ts:763`, `node/ipc.rs:75-79` |
| J2.8 | Database wiped, ledger has people | Setup burns the link and fails with "already has people" on every restart | broken | important | `ui/api/auth.rs`, `ledger/validity.rs` |
| J2.9 | Setup link written to journald (stdout) | Anyone with journal access during the 30 minutes can take the network | untested | later | `kith-ui/src/main.rs:142` |

### J3. Sign-in and sessions

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J3.1 | Passkey sign-in, 12 h sliding session, stored hashed, survives a restart | as designed | works | — | `ui/console.rs:328-363`; `sessions_slide_for_twelve_hours_and_end_on_sign_out` |
| J3.2 | A page left open on the live stream | The stream does not slide the session, so it expires silently; the next click sends you to sign-in and loses the form | broken | important | `web/lib/live/handle.svelte.ts:168`, `web/App.svelte:146` |
| J3.3 | Signed out, or removed, in another tab or by another admin | **The event stream keeps sending** the journal, traffic and join requests (with IP addresses) until it reconnects. The session is checked only when the stream opens | broken | blocker | `ui/stream.rs` |
| J3.4 | Removed person with a leftover session | Read routes (`/api/events`, `/history/*`, peers, traffic, pending) don't re-check the ledger, and the session keeps sliding | broken | blocker | `ui/session.rs:381-386`, `ui/api/people.rs:455` |
| J3.5 | No rate limit on `/api/login/*`; each `begin` writes a row | | missing | later | `ui/api/auth.rs` |
| J3.6 | Local mode (`kith ui`): one-time link, session until the process stops | as documented | works | — | `ui/session.rs` |
| J3.7 | Local `kith ui` with a corrupt history database | Refuses to start, so a broken history file blocks the whole local UI | broken | important | `kith-ui/src/main.rs` (`open_store`) |

### J4. People

Ideal: invite by name and role, share a link or QR; they make a passkey; you confirm. Remove someone, and their devices go with them, everywhere. Lose a passkey: add another from a signed-in device, or have an owner re-issue one, or use the recovery words.

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J4.1 | Invite a member or admin; 7-day link; the inviter confirms admins | works; the page never offers "owner" | works | — | `ui/api/people.rs:197-233`; `people_join_change_roles_and_leave_by_passkey`, `an_admin_invite_works_only_once_its_inviter_confirms_it` |
| J4.2 | A second owner | The server accepts it; the page never offers it, and owners can't be promoted to from the page | missing | important | `web/lib/people.ts` (`addableRoles`, `roleChoices`) |
| J4.3 | Remove a person and their devices | `RemoveAccount` removes every device assigned to them, phones too, and ends their sessions on this Console. Unassigned devices stay (see J5.8). The effect on WireGuard peers is not tested end to end | untested | important | `ledger/validity.rs:754-777`; `removing_a_person_ends_their_approvals_actions_and_devices` (ledger only) |
| J4.4 | Demote an admin | Writes re-check the live role, and so does the ledger. The demoted person's page keeps the admin view until something refreshes status | broken | important | `ui/session.rs:381`, `web/lib/viewer.ts` |
| J4.5 | Last owner: remove or demote | The ledger refuses; the Console gives a generic 400 | works | — | `ledger/validity.rs:435-446`; `roles_decide_who_changes_people` |
| J4.6 | Lost passkey | There is no `AddCredential` op. The only way is a new account and removing the old, which **removes all their devices**. A sole owner can't be replaced | missing | blocker | `ledger/op.rs:64-221` |
| J4.7 | Recovery words | The ledger accepts them and IPC has `RecoverAuthority`, but no CLI or page uses them. Since `AddAccount` needs a passkey made at the Console's domain, recovery must be a Console page: make a passkey, type the words | missing | blocker | `node/ipc.rs:364-379`, `ipc/message.rs:303` |
| J4.8 | Every owner loses their passkey | People, roles and consoles freeze. Admin devices can still make everyday changes | missing | blocker | `node/daemon/people.rs:133-150` |
| J4.9 | People list with two admins at once | No live updates (a 10 s poll only while someone is invited) | broken | later | `web/lib/people.ts` |
| J4.10 | Signature counter (cloned authenticator) | Ignored | missing | later | `kith-crypto/src/webauthn.rs:78-102` |

### J5. Add a Mac or Linux device by code (Console)

Ideal: in the Console, "Add a computer" shows `kith join https://vpn.example.com/j/ABC123` (10 min). On the new device, the command joins, prints "Joined *home* as *alice-mbp*, owned by Alice", and the Console's sheet turns green with that device's name. Any member may do this for their own devices.

Path today: `POST /api/device-codes` (`ui/api/devicecodes.rs:32-83`) → `kith join` fetches `/j/CODE` (`cli/fetch.rs:31`, over HTTPS only) → the Console marks the code used, then mints a one-use invite valid 10 min, acting as the code's creator (`ui/console.rs:668`) → the device joins over the control channel; the sponsor checks expiry on its own clock (`node/network.rs:284`, `ledger/validity.rs:824`) → `DeviceJoined` → `assign_joined` assigns the device if **exactly one** fetched code in that network is still without a device (`ui/console.rs:571-594`).

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J5.1 | First device in a fresh network | Fixed in 9cf5f8e: the first join is now a `DeviceJoined`. Only a unit test on the journal; no e2e from the sponsor's side | untested | important | `node/journal.rs`, `node/daemon.rs:2196`; `the_first_device_in_an_empty_network_is_recorded_as_joining` |
| J5.2 | Nth device | Joins and is assigned. `assign_joined` has no test | untested | important | `ui/console.rs:571-594` |
| J5.3 | **A member adds their own device** | `POST` is allowed, but `GET /j` mints the invite as the member, and the daemon's `is_admin` refuses (needs an owner or admin actor). The code is already burned; the CLI says "wrong or expired". The page hides "Add device" from members anyway | broken | blocker | `node/daemon.rs:833, 1047-1063`, `ui/api/devicecodes.rs:108`, `web/sheets/AddDevice.svelte:243` |
| J5.4 | **Wrong person gets the device** | Any `DeviceJoined` in the window (a phone, a plain invite join, an approved request) is assigned to the code's person, and the real computer stays unassigned | broken | blocker | `ui/console.rs:573-584`, `ui/api/phones.rs:87` |
| J5.5 | **Code burned before anything can fail** | `/j` marks the code used before minting. The CLI fetches before checking its own daemon. So "daemon not running", "no permission", "already joined", "Console's daemon down" and "member" all burn the code, and the stale fetched code then captures the next device (J5.4) | broken | blocker | `ui/api/devicecodes.rs:108-113`, `cli/exec.rs:449-450` |
| J5.6 | Two codes at once (two people, or one person two devices) | **Neither** device is assigned | broken | important | `ui/console.rs:575, 584` |
| J5.7 | Assignment fails (daemon error) | The error is ignored, but the device is stored on the code, so it is never retried | broken | important | `ui/console.rs:587-592` |
| J5.8 | Assign an existing device to a person | IPC `AssignDevice` exists; no CLI, no HTTP route, no page (finding #5) | missing | important | `ipc/message.rs`, `ui/lib.rs` routes |
| J5.9 | Expired code | `/j` answers 410. A fetched invite gets its own 10 min, so a code works up to ~20 min after creation while the page counts from creation and shows "expired" | works | later | `web/sheets/DeviceCode.svelte:52`; `a_device_code_hands_out_one_invite_within_ten_minutes` |
| J5.10 | Reused code; reused invite | 410 `session_used`; "unknown or used-up invite". The CLI prints only the status line | works | later | same test, `node/network.rs:283` |
| J5.11 | Device already in another network | Allowed | works | — | `node/daemon.rs:856` (but see J19: the second network breaks routing) |
| J5.12 | Device already in this network | "already joined", after burning the code (J5.5) | broken | important | `cli/exec.rs:449` |
| J5.13 | Name already taken | Kept in the ledger; the DNS label gets a hex suffix (`work-mac-1a2b`). The Console's validator suggests `name-2`, which is not what happens. When the first leaves, the second takes the plain name | broken | important | `kith-control/src/names.rs:43-63`, `ui/api/validate.rs:66`; `devices_asking_for_one_name_each_get_their_own` |
| J5.14 | Console's daemon down during a join | `/j` burns the code, then fails; the CLI says "wrong or expired" | broken | blocker | `ui/api/devicecodes.rs:108-113` |
| J5.15 | Console reachable only over IPv6 | The control listener binds `0.0.0.0` only, so an IPv6-only server can't sponsor | broken | important | `node/daemon.rs:125, 532` |
| J5.16 | New device behind NAT | Fine; it dials out over TCP | works | — | `invites_lead_with_the_public_hostname_and_joining_through_it_works` |
| J5.17 | What the user sees | CLI: "joined network <id> as <label>" (the network name is fetched and dropped). The page shows **any** `device_joined` as "X joined · Ana's device", without checking it was assigned | broken | important | `web/sheets/DeviceCode.svelte:56-61` |
| J5.18 | Local-mode codes | Links are `http://localhost`, which the CLI refuses | broken | later | `ui/console.rs:147`, `cli/fetch.rs:56` |
| J5.19 | Code guessing | 30 bits, 10 tries/min per **exact** IP; no /64 grouping, no global cap; the limiter map grows without bound | broken | important | `ui/console.rs:439` |

### J6. Add a device by invite or request code (CLI, no Console)

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J6.1 | `kith invite` → `kith join kith://join/…` | works | works | — | e2e |
| J6.2 | `kith invite list` / `revoke` | works; `invite --help` prints a stray "`kith relay …`" heading; networks named `list` or `revoke` can't be invited to | broken | important | `cli/cli.rs:278-290` |
| J6.3 | `kith request-code` → `kith request` → `approve`/`deny` | works; the check code shows on the web page but `kith request` doesn't print it | works | later | `cli/admission.rs` |
| J6.4 | Approved device's owner | Never assigned, from CLI or Console | missing | important | `ui/api/joins.rs:208` |
| J6.5 | Requester clock more than 60 s fast, or hours off | "expired; ask again", forever | broken | important | `node/admissions.rs:25, 259` |
| J6.6 | Request code from the Console | API exists, no page | missing | later | `GET …/request-code` |

### J7. Add a phone or a Windows PC (plain WireGuard)

Ideal: "Add a phone" asks for a name and the owner (default: me). The gateway is the Console server (always on). The config sends all traffic through the server when it is an exit, and only the mesh otherwise. Scan the QR, done. Every member sees "alice-phone (Alice)" as the source of its connections. Removing it drops it at the gateway at once.

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J7.1 | Identity and address | `client_id` of the key, fixed v4/v6, `AddClient` signed by the local device (or `Acted`) | works | — | `node/network.rs:199-217`; `kith-control/src/tests.rs:489-589` |
| J7.2 | **Phone QR in the public Console** | The reveal always asks the OS (`pkcheck`) and Linux servers have no polkit agent, so it answers 403 **after the phone is already in the ledger** | broken | blocker | `ui/api/phones.rs:131-137`, `ui/osauth.rs:115-153`, `ui/api.rs:239` |
| J7.3 | **Gateway** | Always the device running the export or `kith-ui`. `kith ui` on a laptop makes the laptop the gateway, so the phone dies when the laptop sleeps. No choice, no change later, no failover. The gateway's key is in local state only | missing | blocker | `node/daemon.rs:1204-1262`, `node/external.rs:226` |
| J7.4 | **Default route** | The Console starts on "Just my devices". "All traffic" is only offered when the gateway itself is an exit. The exit list drops the local device, so the server Console shows "No exit yet" | broken | blocker | `web/sheets/AddPhone.svelte:95`, `ui/api/exits.rs:53-55` |
| J7.5 | **"No internet until the tunnel is toggled"** (README known issue) | Not fixed. Likely causes: `DNS =` the mesh resolver even in mesh-only mode; an endpoint frozen at export (maybe IPv6); the gateway can't start a handshake after it restarts | broken | blocker | `node/external.rs:318-345`, `node/daemon.rs:1965-1967` |
| J7.6 | Endpoint in the config | `public_hostname`, else the first public address (can be IPv6), else `0.0.0.0:port` | broken | important | `node/daemon.rs:1296-1330` |
| J7.7 | Owner | Assigned when a signed-in person makes it (best effort, errors ignored). CLI and local UI: none | untested | important | `ui/api/phones.rs:79-89` |
| J7.8 | Name | Labels de-duplicated; rename works | untested | later | `node/daemon/manage.rs:75-99` |
| J7.9 | Remove | The gateway drops the peer and session at once | works | — | `a_phone_on_plain_wireguard_is_a_full_member` |
| J7.10 | Policy and attribution | The gateway enforces inbound; the destination enforces outbound; others see the phone's own address; `kith blocked` names it | works | — | same test, `node/blocked.rs:237` |
| J7.11 | Gateway removed or lost | The phone is hidden (`ClientWithoutGateway`); its `AddClient` and address stay; it can only be removed by full id | broken | important | `kith-control/src/netmap.rs:57, 451-468` |
| J7.12 | Address collision from a concurrent `AddClient` | The later phone silently disappears | broken | later | `node/daemon.rs:1265-1272` |
| J7.13 | Show the config again after a Console restart | 404 "remove the phone and add it again" | broken | important | `ui/api/phones.rs:23, 92` |
| J7.14 | Windows | Nothing specific; works as a phone. No docs | missing | important | — |
| J7.15 | Phones reach advertised subnets (`route/…`) | Mesh-only AllowedIPs leave them out | broken | later | `node/external.rs:318-345` |

### J8. Routers

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J8.1 | OpenWrt full node | procd init runs `kith daemon`; MIPS needs a source build | untested | later | `packaging/openwrt/kith` |
| J8.2 | Subnet routes (`route/<prefix>` records) | Work for kith devices | untested | later | `kith-control/src/records.rs:29, 77`, `node/os.rs:97-105` |
| J8.3 | Plain-WireGuard router advertising a LAN | Not possible | missing | later | `ledger/op.rs` (`AddClient`) |

### J9. Exits

Ideal: an admin marks the server as an exit (done by `--console`). Anyone picks it. If it goes offline or is removed, the device says so at once and offers "turn off exit". DNS goes through it with no leaks. IPv6 is carried or blocked, never leaked.

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J9.1 | Allow (admin record), use (local, saved) | works | works | — | `node/daemon.rs:756-763, 968-996`; `exit_traffic_goes_out_and_back_and_the_choice_survives_a_restart` |
| J9.2 | Switch, turn off | works; switching has no test | untested | later | `node/daemon.rs:981-985` |
| J9.3 | Exit goes offline | Fails closed (good). Nothing tells the user except `kith doctor` | untested | important | `sys/linux.rs:53-66`, `sys/macos.rs:117-137`, `cli/doctor.rs:80-96` |
| J9.4 | **Exit removed by another admin** | The device keeps routing into it. Status hides it because the name lookup fails, so doctor says nothing. Silent blackhole | broken | blocker | `node/daemon.rs:1366-1369` |
| J9.5 | Exit grant revoked | The client keeps using it until restart; the exit never checks the grant | broken | important | `node/daemon.rs` |
| J9.6 | DNS through the exit, macOS | All services' DNS overridden; split DNS via `/etc/resolver` | untested | — | `sys/macos.rs:213-266` |
| J9.7 | DNS through the exit, Linux with resolved | `~.` and `default-route` on kith's link; other links can still get queries | untested | important | `sys/linux.rs:209-224` |
| J9.8 | DNS, Linux without resolved | See J1.2 | broken | blocker | |
| J9.9 | Exit without IPv6 | Client IPv6 goes down the tunnel and is dropped: no leak, but no IPv6 | untested | later | `sys/linux.rs:77-83` |
| J9.10 | macOS on an IPv4-only network, exit with an IPv6 address | `route get` fails and the whole plan rolls back | broken | important | `sys/macos.rs:126-130` |
| J9.11 | Exit NAT after reboot | Re-applied, but only if `[server] exit` is in config (see J1.11) | broken | blocker | `sys/linux.rs:73-97` |
| J9.12 | Config `exit.use_exit`, `exit.allow_lan`, `dns.override_system` | Parsed, never read | broken | later | `kith-config/src/schema.rs` |
| J9.13 | Console server's own exit changed by an admin | Allowed with no OS prompt; may cut the Console off | untested | important | `/api/exit` |

### J10. Relays and connectivity

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J10.1 | Direct UDP blocked → relay | Automatic | works | — | `traffic_falls_back_to_the_relay_and_returns_to_direct` |
| J10.2 | All UDP blocked | The relay is UDP. QUIC only if the server sets `quic_listen`; WebSocket needs URLs and TLS. All manual | missing | important | `node/stealth.rs:76-119` |
| J10.3 | Relay down, two NATed laptops reconnecting | Existing direct paths survive; new hole punching needs the relay, so they likely can't connect | untested | important | `node/connectivity.rs:339-380` |
| J10.4 | Relay advertised at IPv6 only | See J1.10 | broken | blocker | |
| J10.5 | Keepalive to every peer every 10 s | n² traffic, all through the relay when relayed | broken | important | `node/daemon.rs:56, 1931` |

### J11. Remove, rename, leave

Ideal: removing a device, online or offline, cuts it off everywhere, and when it next comes up it learns "you were removed from *home*", cleans up and says so. Leaving signs a removal. The last admin can't leave by accident.

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J11.1 | Remove an online device | Others cut it off | works | — | `an_admin_removes_a_device_and_it_is_cut_off` |
| J11.2 | **The removed device itself** (online or offline) | Never learns: peers close with `NotMember` before syncing. It keeps its routes and DNS, shows every peer offline, and dials forever | missing | important | `kith-channel/src/connection.rs:441-447`, `node/channel.rs:470-480` |
| J11.3 | Remove the last device | Recorded since 1006729 (journal only). Leaving your last network probably logs every former peer as removed | untested | later | `node/journal.rs:149-158` |
| J11.4 | **Remove the Console's own device** | Becomes "leave" (local only, behind an OS prompt that fails on a server). Everyone else keeps it as a member forever | broken | important | `ui/api/devices.rs:109-146`, `node/daemon.rs:913-929` |
| J11.5 | **Leave** | Local only; signs nothing, so the network keeps a ghost member with a trusted key | broken | important | `node/daemon.rs:913-929` |
| J11.6 | **The only admin leaves** | No guard. Nobody can sign admin ops again | broken | blocker | `node/daemon.rs:913`, `cli/exec.rs:189` |
| J11.7 | Rename offline device | Synced later | works | — | `an_admin_renames_a_device_and_every_member_sees_it` |
| J11.8 | Rename to a taken name | Accepted; the older device keeps the plain label, so "nas" silently moves | broken | important | `kith-control/src/names.rs:43-63` |
| J11.9 | Rename this device from the CLI | Not possible: lookup only searches peers | broken | important | `cli/exec.rs:540-560` |
| J11.10 | Console's own device removed while signed in | The page goes blank (404 boot, no gate); with several networks it silently switches network | broken | important | `ui/console.rs:250-268`, `web/lib/live/core.svelte.ts:347` |

### J12. Restarts and network changes

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J12.1 | Daemon restart | Reloads identity, ledgers, exit, imports, caches; routes re-applied | works | — | `restarted_nodes_reconnect_from_their_saved_state` |
| J12.2 | Server reboot | Units enabled; roles lost (J1.11) | broken | blocker | |
| J12.3 | Console restart | Sessions, prompts, codes, journal kept; phone configs and recovery words lost | works | important | `ui/store/sqlite.rs`, J2.4, J7.13 |
| J12.4 | Daemon down under the Console | Banner after 3 s, changes blocked, resync on `daemon_up`. Copy says "sudo kith up" / "on this device" in public mode | works | later | `web/lib/copy.ts:705-708` |
| J12.5 | **Main loop stops** (dataplane ends, or a panic in a non-dist build) | `run` waits only for a signal, so the process stays up doing nothing and the service manager never restarts it | broken | blocker | `node/daemon.rs:283-287, 637-641` |
| J12.6 | Crash, then restart | Nothing undoes leftover macOS DNS, resolver files and routes at start; if the daemon then can't start, DNS stays pointed at the dead resolver | broken | important | `sys/macos.rs:99-185` |
| J12.7 | Mac sleep/wake | Nothing detects wake; peers look online for ~25 s; last-seen off by the sleep length | untested | important | `node/daemon.rs:2752` |
| J12.8 | Wi-Fi → mobile | Netmon polls every 5 s, re-STUNs, re-punches; never rebinds; the macOS host route to the exit keeps the old gateway | broken | important | `kith-netmon/src/snapshot.rs:17`, `node/connectivity.rs:521-536`, `node/daemon.rs:2110` |
| J12.9 | `kith down` stays off across reboot | yes | works | — | `cli/service.rs:83-102` |

### J13. Access policy

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J13.1 | Default allow-all | as designed | works | — | `kith-policy/src/policy.rs:287` |
| J13.2 | Blocked connection recorded and allowed with one rule | works | works | — | `a_blocked_connection_is_reported_and_allowed_with_one_rule` |
| J13.3 | Deny-all | Allowed; only tunnel traffic stops; management keeps working | untested | later | `node/network.rs:243` |
| J13.4 | Rules by person ("Alice's devices") | No person selector | missing | important | `kith-policy/src/policy.rs:81-89` |
| J13.5 | Change a device's tags after it joins | No op | missing | important | `ledger/op.rs` |
| J13.6 | Two admins edit the policy at once | One whole-policy record, last writer wins; one edit is lost | broken | later | `node/daemon.rs:2367-2372` |
| J13.7 | Allow to or from this device (CLI) | Can't name this device | broken | important | `cli/exec.rs:540-560` |

### J14. DNS names

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J14.1 | `<device>.<network>.kith.internal` | works | works | — | `devices_asking_for_one_name_each_get_their_own` |
| J14.2 | Network name with spaces ("My Home") | Never cleaned; DNS silently off for that network | broken | important | `ledger/op.rs:769`, `node/dns.rs:169` |
| J14.3 | Two networks with the same name | `DuplicateSuffix` aborts every refresh | broken | important | `kith-naming/src/zone.rs:127`, `node/daemon.rs:1841` |
| J14.4 | A CNAME on a device's name | Drops all custom records of that network | broken | later | `node/dns.rs:136-141` |

### J15. Upgrades and version skew

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J15.1 | **Mixed versions across devices** | No negotiation. Channel, sync and op versions are all 1, yet new channel messages and `OpBody` variants (accounts) were added after v0.1.1. Old nodes drop the connection or orphan every later op; sync never settles; nobody is told | missing | blocker | `kith-channel/src/message.rs:13`, `kith-sync/src/lib.rs:63`, `kith-sync/src/session.rs:250-253`, `ledger/op.rs:42` |
| J15.2 | Downgrade with an unknown op on disk | The daemon won't start | broken | important | `node/state.rs:61-65` |
| J15.3 | CLI or kith-ui vs daemon | Exact IPC version match; clear 503 `version_skew` | works | — | `ui/ipc.rs:65` |
| J15.4 | Upgrade path | Re-run install.sh; doesn't restart (J1.5). No `kith upgrade` | broken | blocker | |
| J15.5 | Console SQLite migrations | Runner works; an older Console opening a newer DB is not refused | broken | later | `ui/store/sqlite.rs:80` |

### J16. Clock skew

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J16.1 | Invite expiry (inviter's clock) vs join time (sponsor's clock) | No tolerance; hours of skew make a valid invite look expired, or an old one still work | untested | important | `ledger/validity.rs:825` |
| J16.2 | Join requests | 60 s ahead / 10 min old, by the admin's clock; misleading error (J6.5) | broken | important | `node/admissions.rs:25` |
| J16.3 | Ledger order, control TLS | Lamport clocks; TLS ignores time | works | — | `ledger/validity.rs:58` |

### J17. Tailscale or another VPN present

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J17.1 | **Same defaults as Tailscale** | kith uses 100.64.0.0/10, Tailscale's own IPv6 prefix `fd7a:115c:a1e0::/48`, and UDP 41641. With Tailscale running, kith fails to start (clear message naming the port) | broken | blocker | `node/network.rs:48-53`, `kith-config/src/schema.rs:182`, `node/types.rs:31-34` |
| J17.2 | Choose the range at create | Not possible | missing | blocker | `node/network.rs:48` |
| J17.3 | Route overlap detection | Warns in status and doctor, but only when kith's own plan changes; Tailscale started later goes unnoticed | broken | important | `sys/conflicts.rs:41`, `node/os.rs:155-175` |

### J18. A laptop with kith off for work

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J18.1 | `kith down` reverts routes and DNS | Clean on SIGTERM | works | — | `node/daemon.rs:282, 2117-2121` |
| J18.2 | Pause for N hours | `paused_until` always `None` | missing | important | `ui/api/status.rs:336` |
| J18.3 | Turn kith on or off without the CLI | No control anywhere (owner feedback) | missing | important | `web/lib/copy.ts:705-708` |
| J18.4 | Back after days | Shows offline to others; syncs on return | works | — | `a_silent_peer_goes_offline_quickly…` |

### J19. Several networks on one device

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J19.1 | **Join a second network** | Every network gets the same 100.64/10 range; the router refuses overlapping mesh ranges, so routing for the second fails. The join is saved first, so on restart the daemon won't start. The README promises "each with its own addresses" | broken | blocker | `node/network.rs:48-52`, `kith-router/src/router.rs:209-211`, `node/daemon.rs:1794-1796` |

### J20. Daemon robustness

| ID | Case | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| J20.1 | No config / bad config | Defaults / clear error (no file path in it) | works | later | `kith-config/src/error.rs:27` |
| J20.2 | Crash mid-write | Temp, fsync, rename, CRC | works | — | `kith-store/src/fs.rs:135-160` |
| J20.3 | **One corrupt ledger or cache file** | The whole daemon refuses to start (contrary to the comments). No command helps | broken | blocker | `node/state.rs:58-65, 162, 192, 225` |
| J20.4 | Disk full while saving | In memory, retried on the next save; a join in that window is lost on restart | untested | later | `node/daemon.rs:677-693, 900-901` |
| J20.5 | Port taken | Fails with a clear message; no fallback | works | — | `a_taken_port_fails_with_an_error_that_names_it` |
| J20.6 | Hundreds of devices | Link reports and endpoint gossip capped at 256 (the rest silently dropped); ledger file capped at 16 MiB (then saves fail forever); keepalives n² | broken | important | `node/daemon.rs:51`, `kith-store/src/format.rs:6` |
| J20.7 | Long uptime | Journal, blocked list, admissions, topology bounded; endpoint cache never pruned; ledger never compacted | untested | later | `node/journal.rs:12-16` |
| J20.8 | macOS log | `/var/log/kith.log`, never rotated | broken | important | `packaging/launchd/dev.kith.daemon.plist:24` |

## 4. Surfaces

### CLI (every command)

The CLI parses every command, has help, and with no daemon prints one clear error and exits 1 (`cli/main.rs:76`); all failures exit 1. `--json` exists only on `status`, `peers` and `dns list`. Without the operator group even `status` is refused at the socket (0660), so the daemon's "anyone may read" rule does not reach normal users.

| ID | Command | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| C1 | Device ids | The 8-character ids that `peers` and `status` print are refused by `remove`, `rename`, `exit use/allow/deny`, `allow`, `relay --node` | broken | important | `cli/render.rs:141`, `cli/exec.rs:106-128` |
| C2 | This device by name | Not found by `rename`, `allow`, `remove` (wrong hint) | broken | important | `cli/exec.rs:540-560` |
| C3 | `kith server --exit` | Silently turns relay off (replaces both modes); our own hint tells users to run it | broken | important | `node/daemon.rs:1182`, `cli/render.rs:556` |
| C4 | Help text | `invite`, `exit`, `dns`, `wg`, `config` show stray doc-comment headings; `dns set` doesn't explain TYPE | broken | important | `cli/cli.rs:208, 242, 278, 379, 424` |
| C5 | No-daemon hints | Say `kith daemon` / `sudo kith daemon`, should say `kith up` | broken | important | `cli/exec.rs:387`, `cli/doctor.rs:258` |
| C6 | Daemon errors | Internal prefixes ("invalid argument: …", 64-char ids); admin refusals classed as invalid argument | broken | important | `node/types.rs:42-46`, `node/daemon/manage.rs:211` |
| C7 | Arguments checked late | Bad DNS values or unreadable files report "cannot reach the daemon" when it is down | broken | later | `cli/exec.rs:411-414` |
| C8 | Exit codes | Always 1; scripts can't tell usage from daemon-down | broken | later | `cli/main.rs:76` |
| C9 | `--json` on invite, requests, people, blocked, doctor, map | | missing | later | |
| C10 | Operator group name in the hint | Hard-coded, though configurable | broken | later | `cli/exec.rs:383` |
| C11 | Flag names | `request --name` vs `create`/`join --device-name`; docs/server.md uses `join --name` | broken | later | `docs/server.md:75` |
| C12 | `exit off` | Asks for a network when in several | broken | later | |
| C13 | `wg` name lookup | Takes the first match, no ambiguity check | broken | later | `cli/exec.rs:91` |
| C14 | `doctor` with a bad kith.toml | Gives up instead of reporting it | broken | later | `cli/exec.rs:478` |
| C15 | `status`, `create`, `join`, `leave`, `peers`, `people`, `map`, `dns`, `relay`, `watch`, `log`, `top`, `blocked`, `config`, `ui` | as documented | works | — | `cli/tests.rs` |
| C16 | `up`, `down`, `daemon` | Need root; not run | untested | — | `cli/service.rs` |
| C17 | `kith assign`, `kith pause`, `kith upgrade`, `kith uninstall` | | missing | important | |

### Console and local UI

| ID | Surface | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| U1 | Routes, Overview, Needs-you, Activity, Exits, Access, Shortcuts, Gate | works | works | — | `boot.spec`, `keyboard.spec`, `history.spec`, `manage.spec`, `router.test.ts` |
| U2 | Devices: rename, remove; Names; deny; leave; Welcome create/join | No end-to-end test | untested | important | `a11y.spec` renders only |
| U3 | People, Setup, Register, Login, DeviceCode | Happy paths | works | — | `console.spec`, `console_tests.rs` |
| U4 | **Who sees what** | The page takes the role from the Console's *device*, not the person: an owner shows as "Admin", members get exact link rates and pending joins with IPs | broken | important | `web/frame/net.ts:21`, `ui/api/status.rs:279, 464` |
| U5 | Buttons members see that answer 403 | Use an exit, import WireGuard, offer this device, leave network | broken | important | `ui/session.rs:52` |
| U6 | Tier-3 actions in public mode (offer exit/relay, leave, phone config, import) | Fail after a wait on a headless server; as root, polkit lets them through with no prompt | broken | blocker | `ui/osauth.rs:135` |
| U7 | "This device" on a server-hosted page; copy written for local mode ("Run kith ui to reopen it", "sudo kith up", "Your internet now goes through …") | Misleads remote admins | broken | important | `web/lib/copy.ts:41, 705, 763`, `web/App.svelte:311-313` |
| U8 | One dashboard for the network and the device it is opened from | Not built | missing | later | `notes/ui-feedback.md` |
| U9 | APIs without a page: assign device, authorize/revoke console, recovery, request code | | missing | important | |
| U10 | ACME failure or renewal failure | Silent; HTTPS fails; port 80 redirects to it | broken | important | `ui/public.rs` |
| U11 | SQLite errors at runtime | Session lookup errors sign everyone out silently; `console_network` writes a setting on every request | broken | later | `ui/console.rs:262, 350` |
| U12 | Slow networks | SSE backoff fine; fetches have no timeout | broken | later | |
| U13 | Phone width, dark mode, a11y | Axe at 360–1920 px in both themes for local screens; Console pages (People, Login, Setup, Register, DeviceCode) not audited; sign-in pages ignore the theme setting | untested | important | `a11y.spec`, `tokens.css:203-229` |

### Security

| ID | Item | Now | Status | Sev | Refs |
|---|---|---|---|---|---|
| S1 | **The Console server is a permanent admin** | As the genesis admin device (threshold 1) it signs every admin op with no person and can `AddAdmin` any key. `RevokeConsole` and `RemoveDevice` don't take that away, and owners can't fix it. Root on the server bypasses passkeys | broken | blocker | `ledger/validity.rs:341, 680-689` |
| S2 | `actor` is trusted | Any operator on the Console host can name any person as actor; nothing proves they asked | untested | important | `node/daemon/people.rs:46`, `node/network.rs:259` |
| S3 | Passkey signs an opaque op hash | A malicious Console can swap the op behind an owner's prompt (e.g. add an attacker owner). The web model can't fully fix this; document it, and show the decoded change | missing | important | `ui/api/auth.rs` |
| S4 | Host, Origin, CSP, cookies, DNS rebinding | Sound in both modes | works | — | `ui/session.rs:314-354`, `tests/router.rs` |
| S5 | Stream and reads after removal | See J3.3, J3.4 | broken | blocker | |
| S6 | Code brute force | See J5.19 | broken | important | |
| S7 | Recovery words known to the server during setup | By design today; document | untested | later | `ui/api/auth.rs` |
| S8 | `RevokeConsole`: the page needs an owner, the ledger allows admins | | broken | later | `ui/api/people.rs:308` |
| S9 | Admins in the public Console can make the server join, create or leave networks and change its exit, with no prompt | | broken | important | `/api/join`, `/api/networks`, `/api/exit` |
| S10 | Invite backdating by a member | Known gap | missing | later | `ledger/validity.rs:824` |

### Docs and copy

| ID | Where | Claim | Truth | Status | Sev |
|---|---|---|---|---|---|
| D1 | `docs/security.md` whole | No people, no public Console, no compromised-Console threat | The Console exists; S1–S3 | broken | blocker |
| D2 | `security.md:63, 74` | Invites can't be revoked | `kith invite revoke`, `/invites/<id>/revoke` | broken | important |
| D3 | `security.md:76` | No recovery if admins are lost | Recovery words exist (but unusable, J4.7) | broken | important |
| D4 | `security.md:80` | Web sessions never expire | True for local only; Console sessions are 12 h | broken | important |
| D5 | `security.md:46-53`, README:60 | The web page is 127.0.0.1-only, nothing on disk | Public mode; SQLite history on disk | broken | important |
| D6 | README:76 | "Several networks at once, each with its own addresses" | J19.1 | broken | blocker |
| D7 | README:9-68, 88; security.md:67, 78 | No Console, "one admin" | Console with people and roles | broken | important |
| D8 | `docs/server.md:7-12, 33, 54-75` | Build on the VPS; `join --name`; `ws_listen :443` | install.sh; `--device-name`; clashes with the Console | broken | important |
| D9 | `docs/node.md:62-89` | State layout, module table, IPC authorization | Missing files, modules and the operator group | broken | later |
| D10 | `docs/architecture.md:189, 224-232, 286` | 36 crates, coordinator plan, dependency table | 38 crates, Console replaced it, deps not in Cargo.lock | broken | later |
| D11 | `docs/roadmap.md:11-15` | Items already done | | broken | later |
| D12 | `docs/plans/console.md:31` | Local passkeys work | Local mode has no setup link | broken | later |
| D13 | `packaging/README.md:15` | Unit capabilities | Also CAP_CHOWN | broken | later |

## 5. Totals

| | Rows | Works | Untested | Broken | Missing | Blocker | Important |
|---|---|---|---|---|---|---|---|
| Journeys J1–J20 | 157 | 33 | 23 | 77 | 24 | 35 | 65 |
| CLI | 17 | 1 | 1 | 13 | 2 | 0 | 7 |
| Console and UI | 13 | 2 | 2 | 7 | 2 | 1 | 7 |
| Security | 10 | 1 | 2 | 5 | 2 | 2 | 4 |
| Docs | 13 | 0 | 0 | 13 | 0 | 2 | 6 |
| **All** | **210** | **37** | **28** | **115** | **30** | **40** | **89** |

Some rows share a root cause (J1.11, J9.11 and J12.2; J3.3 and S5), so the 40 blocker rows come down to 26 blocker fixes in the plan below.

## 6. Fix plan: three independent workstreams

Ownership is by file so that nobody edits the same file in parallel. Where one stream needs something from another, it is listed as a dependency and lands as its own small change first.

### WS1: Mesh core (daemon, routing, sync, OS)

**Owns:** `crates/management/kith-node` (all but `src/daemon/people.rs`), `kith-ipc` (other streams may only append variants at the end of the enums, with one protocol bump per release), `kith-store`, `kith-config`, `crates/control/*`, `crates/data/*`, `crates/connectivity/*`, `crates/naming/*`, `crates/platform/kith-sys`, `crates/trust/kith-policy`.

| # | Fix | Rows | Sev |
|---|---|---|---|
| 1 | Give each network its own ranges: pick a free random /16 in 100.64/10 and a random ULA /48 at create (never Tailscale's `fd7a:115c:a1e0::/48`), with `kith create --range`. On join, refuse a clash clearly instead of saving a network that can't route | J19.1, J17.1, J17.2, D6 | blocker |
| 2 | Default listen port away from 41641 (e.g. 51820+offset); if the port is taken, pick a free one and advertise it | J17.1, J1.4 | blocker |
| 3 | Protocol versions in the channel handshake. Refuse a peer that's too old with "upgrade kith on X", and surface it in status and the journal. Make sync skip ops it can't decode without orphaning the rest; reserve a "future op" path | J15.1, J15.2 | blocker |
| 4 | `run` returns when the main loop ends, so systemd and launchd restart it. At start, undo any leftover OS plan from a crash | J12.5, J12.6 | blocker |
| 5 | A corrupt network ledger or cache file is moved aside with a warning, not a reason to stay down | J20.3 | blocker |
| 6 | Persist server roles set at runtime (`kith server`, `/api/server`) and re-apply at start; `kith server` adds a role instead of replacing both | J1.11, J9.11, J12.2, C3 | blocker |
| 7 | Exit removed or grant revoked: drop it, restore direct routing, raise a journal warning that the UI shows; the exit checks the grant before forwarding | J9.4, J9.5, J9.3 | blocker |
| 8 | Relay and phone endpoints: prefer `public_hostname`, else an IPv4 address; listen on IPv6 too | J1.10, J10.4, J7.6, J5.15 | blocker |
| 9 | Phones: choose the gateway (default: an always-on server that is an exit); offer "all traffic" when any allowed exit exists; mesh-only configs carry no `DNS =` line, or a fallback; the gateway keeps sessions across restart. Write a test that restarts the gateway with a phone attached | J7.3, J7.4, J7.5 | blocker |
| 10 | Leave signs a self-removal (needs WS2 #6). Refuse to leave or remove the last admin. Tell a removed device: peers send one "removed" notice before closing; the device cleans up and says so | J11.5, J11.6, J11.2, J11.4 | blocker / important |
| 11 | Journal `DeviceJoined` carries the invite key that admitted it (for WS2 #2) | J5.4 | blocker |
| 12 | Debian without resolved: write DNS where the unit allows it (with WS3 #2), and don't let a DNS failure roll back routes and NAT | J1.2, J9.8 | blocker |
| 13 | macOS: re-apply the exit host route on network change; don't fail the plan on an IPv6 exit address from an IPv4-only network; rebind on change; detect wake and re-probe | J12.7, J12.8, J9.10 | important |
| 14 | Rename to a taken name refuses (or suffixes `-2` as the Console promises); network names cleaned into DNS labels; duplicate suffixes don't abort refresh | J5.13, J11.8, J14.2, J14.3 | important |
| 15 | Re-check route conflicts periodically; keepalives only to active peers; lift the 256 caps or page them; prune the endpoint cache | J17.3, J10.5, J20.6, J20.7 | important |
| 16 | Admission tolerance of ±5 min and a clear "your clock is off" error; same for invite expiry | J6.5, J16.1, J16.2 | important |
| 17 | Errors without internal prefixes; admin refusals are `PermissionDenied` | C6 | important |
| 18 | Pause with a deadline; owner and tags in `PeerInfo`/`BlockedInfo`; person selectors in policy; a tag-change op (with WS2 #6) | J18.2, J13.4, J13.5 | important |

### WS2: People, Console and web

**Owns:** `crates/trust/kith-ledger`, `crates/management/kith-node/src/daemon/people.rs`, `crates/management/kith-ui-core`, `crates/management/kith-ui` (server and `web/`), `crates/foundation/kith-crypto/src/webauthn.rs`.

| # | Fix | Rows | Sev |
|---|---|---|---|
| 1 | Device codes: mint the invite **before** burning the code, and mint it as the Console (not the member), so members can add their own devices; show "Add device" to every signed-in person | J5.3, J5.5, J5.14 | blocker |
| 2 | Assignment by invite key (needs WS1 #11): bind the code to its invite, assign exactly the device that used it, retry on failure; the page waits for *that* device and shows the owner | J5.4, J5.6, J5.7, J5.17 | blocker |
| 3 | An "Owner" picker on every device row (`AssignDevice` route), also for approved requests and phones from the CLI | J5.8, J6.4, J7.7 | important |
| 4 | Public-mode tier 3: replace the OS prompt with a passkey prompt (or none for the phone QR the person just made) and hide actions that don't belong to a remote admin (leave, offer this device, import) | J7.2, U6, S9, J11.4 | blocker |
| 5 | Setup that can't lose the words: generate and show the words **before** anything is published, require "I saved them", fail setup on any recovery-key error, burn the token last, and let a signed-in owner resume `authorize_console` | J2.2–J2.5, J2.8 | blocker |
| 6 | Ledger: `AddCredential` (a person adds a passkey; an owner re-issues one), owner-driven `RemoveAdmin` and a setup hand-off that demotes the Console from admin device to console only, a self-removal op for leave, a tag-change op | J4.6, S1, J11.5, J13.5 | blocker |
| 7 | A recovery page: make a passkey, type the 18 words, become owner again | J4.7, J4.8 | blocker |
| 8 | The event stream and every read re-check the session and the person on each event or request | J3.3, J3.4, S5 | blocker |
| 9 | Phones page defaults: owner, gateway and "all traffic through the server"; list the local device as an exit; keep phone configs encrypted in SQLite so they can be shown again | J7.4, J7.13 | blocker (with WS1 #9) |
| 10 | Role from the person, not the device: owner shows as Owner; members see bands, not exact rates or join IPs; hide what members can't do; refresh `me` on role change; offer owners in invite and role menus | U4, U5, J4.2, J4.4 | important |
| 11 | Public-mode copy: "the server" instead of "this device"; permission errors say what to do (finding #3); report `operator` truthfully and check it at start | J2.7, U7, J12.4 | important |
| 12 | Console's own device removed: a clear gate instead of a blank page; never switch networks silently | J11.10 | important |
| 13 | ACME events logged and shown on the setup page; port conflicts reported, not crash-looped; one setup link per install (don't rotate on every restart) | J1.8, J1.9, U10 | important |
| 14 | `/j` rate limit per /64 plus a global cap, bounded map; login rate limit | J5.19, J3.5 | important |
| 15 | Sessions slide while the stream is open; keep form input across re-sign-in; fetch timeouts | J3.2, U12 | important |
| 16 | Tests: `assign_joined`, rename/remove/names/deny/leave e2e, Console pages in `a11y.spec`, People live updates | J5.2, U2, U13, J4.9 | important |

### WS3: Install, upgrade, CLI and docs

**Owns:** `install.sh`, `packaging/`, `infra/`, `.github/workflows/`, `crates/management/kith-cli`, `README.md`, `docs/` (all but `docs/plans/journeys.md`, which whoever closes a row updates), `CONTRIBUTING.md`.

| # | Fix | Rows | Sev |
|---|---|---|---|
| 1 | Upgrades: install.sh restarts kith (and kith-ui when present) after replacing binaries, rewrites unit files, upgrades kith-ui whenever it is installed; `kith up` restarts a running service of another version | J1.5, J1.6, J1.7, J15.4 | blocker |
| 2 | `kith.service`: `ReadWritePaths` for resolv.conf and `/run`; installer checks for `nft` and TUN | J1.2, J1.1 | blocker |
| 3 | `--console`: write `server.public_hostname` and `[server] exit = relay = true` to `/etc/kith/kith.toml`; open 41641 (or the new port) and 80/443 where ufw or firewalld runs; check that the domain resolves to this host before starting, and say so | J1.10, J1.11, J1.13, J1.8 | blocker |
| 4 | Ship a release with kith-ui before install.sh's `--console` reaches main | J1.14 | blocker |
| 5 | `kith join https://…`: connect to the local daemon and check "already in this network" **before** fetching the code; print the network name and owner; show the server's message on errors | J5.5, J5.12, J5.17, J5.10 | blocker |
| 6 | Device lookup takes the printed 8-character ids and this device's own name | C1, C2, J11.9, J13.7 | important |
| 7 | Help text, hints (`kith up`), exit codes (2 for usage, 3 for daemon down), `--json` on the remaining list commands, check arguments before connecting | C4, C5, C7, C8, C9 | important |
| 8 | `kith uninstall`, macOS log rotation (newsyslog), `kith assign`, `kith pause` (with WS1 #18) | J1.15, J20.8, C17 | important |
| 9 | Lightsail scripts: use install.sh `--console`, back up `/etc/kith` and `/var/lib/kith-console` | J1.11 | important |
| 10 | Rewrite `docs/security.md` for people and the public Console, including what a compromised Console can do (S1–S3), and fix D2–D13; README: Console section, drop "several networks" until WS1 #1 lands, known issues | D1–D13 | blocker (D1, D6) |
| 11 | Signed and notarized macOS builds; signatures for SHA256SUMS | J1.16 | later |

**Order within each stream:** blockers top to bottom, then important. **Cross-stream dependencies:** WS2 #2 needs WS1 #11; WS1 #10 needs WS2 #6 (self-removal op); WS2 #9 needs WS1 #9; WS3 #8 `kith pause` needs WS1 #18. Each dependency is a small, separate change that lands first.

**Definition of done for launch:** every blocker row is fixed and has a test; the live test is re-run on a fresh Ubuntu VPS, a fresh Debian VPS and a Mac with Tailscale installed, covering J1, J2, J5, J7, J9, J11, J12 and J15 (two versions); this file is updated to match.
