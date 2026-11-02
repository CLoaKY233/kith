# kith-ui: HTTP API between the page and the `kith-ui` server

The page talks only to the `kith-ui` process on the same machine. `kith-ui` talks to the daemon over IPC (`kith-ipc`). JSON everywhere; field names are snake_case. Every type named here is generated into `kith-ui/web/src/lib/types.gen.ts` by `cargo xtask ui-types`, and a test in `kith-ui-core` fails while that file is stale.

This file follows `docs/plans/kith-ui-design.md` §2, which wins where they differ. The server is `kith-ui-core::router`, an axum `Router`.

## Session and guards

- `kith-ui` listens on `127.0.0.1:<port>` (default 41680, `--port` to change; a free port if taken).
- `kith ui` runs `kith-ui`, which prints `http://127.0.0.1:<port>/#t=<token>` and hands it to the OS to open in the default browser (`--no-open` only prints it). The token is 32 random bytes in hex, single-use, valid 2 minutes, compared in constant time and burned on use. Under `--dev` the link points at Vite's origin instead.
- The page POSTs `/api/session` with `SessionIn {token}`. The answer is 204 and sets `kith_ui_<port>=<secret>; HttpOnly; SameSite=Strict; Path=/`: a separate secret, in a cookie named for the port because browsers share cookies across the ports of a host. The page then drops the fragment with `history.replaceState`.
  - A wrong token gets 401 `unauthorized` and burns nothing. Once the token is used, every token gets 401 `session_used`; the right token older than 2 minutes gets 401 `session_expired` and is burned.
  - One link starts one session, and the session lasts until `kith-ui` stops: there is no sign-out and no expiry. A new session needs a new `kith ui`.
- Every request must name this machine in `Host` (`127.0.0.1:<port>` or `localhost:<port>`), or it gets 403 `forbidden`.
- Every request other than GET and HEAD, and every request to `/api/events`, must also come from the page: an `Origin` of `http://127.0.0.1:<port>` or `http://localhost:<port>` (under `--dev`, Vite's origin too). Browsers leave `Origin` off a same-origin `EventSource`, so `/api/events` without `Origin` is allowed when `Sec-Fetch-Site` is `same-origin` or absent. Anything else gets 403 `forbidden`.
- Every response, refusals included, carries:
  - the CSP `default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'`;
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy: same-origin`;
  - under `/api/`, `Cache-Control: no-store` unless the route sets its own.
- Without the cookie, API calls other than `/api/session` get 401 `unauthorized`, and the page routes render the signed-out page with no boot snapshot. With it, the page routes inline the boot snapshot and are served `Cache-Control: no-store`.

## Errors

Status 400, 401, 403, 404, 409, 500 or 503 with `ApiErrorBody {code, message, conflict}`. `code` is an `ApiErrorCode`; `message` is the server's or daemon's words, for logs and the `internal` copy only.

| Status | `code` | When |
|---|---|---|
| 400 | `invalid_argument` | a bad id, body or value; the daemon refused an argument |
| 401 | `unauthorized`, `session_used`, `session_expired` | no session, or the link token was used or lapsed |
| 403 | `forbidden` | wrong `Host` or `Origin` |
| 403 | `permission_denied` | a member asked for an admin's change |
| 403 | `os_auth_denied` | a tier-3 change the OS did not confirm, or this computer cannot ask; nothing was signed |
| 404 | `not_found` | the network, device or name is gone |
| 409 | `conflict` | another device settled it first; see below |
| 500 | `internal` | the daemon answered something unexpected, or another failure; `message` says what |
| 503 | `daemon_down`, `not_in_group`, `version_skew` | the daemon is not running, this OS user may not open its socket, or it speaks another protocol |

A 409 fills `conflict` with `{resolved_by, resolution, at_ms, op}`: who won the race and how. The daemon does not yet say which device made a change, so `resolved_by` is empty and the page names no one.

A path under `/api/` that no route serves gets 404 `not_found`.

## Reads

| Method | Path | Body |
|---|---|---|
| GET | `/api/boot` | `Boot {boot, seq, server_time_ms, status, peers, traffic_tail, pending, versions, last_tick}`. The same object is inlined into `index.html` as `<script type="application/json" id="boot">` |
| GET | `/api/status` | `StatusView`: `Status` plus `operator`, `versions`, `paused_until`, `memberships` (role and admins per network), `host`, `exit_state` |
| GET | `/api/pending` | `Pending {joins: [JoinRequest], blocked: [BlockedGroup]}` for every network this device is a member of. The daemon lists join requests to members too; the page shows them to admins only. A join request carries `id` (`join:<network>:<node>`), `check_code`, `claimed {name, os}` (`os` is always `null` for now), `seen {ip, key_fpr}` (`ip` only when the request reached this device directly), `requested_at_ms` and `expires_at_ms` (10 minutes later). A blocked group carries `count`, `last_ms` and the `rule` that would allow it; the daemon keeps no first time, so `first_ms` equals `last_ms` |
| GET | `/api/traffic` | `[TrafficSeries]` |
| GET | `/api/networks/<id>/peers` | `[PeerInfo]` |
| GET | `/api/networks/<id>/records` | `[RecordView]` |
| GET | `/api/networks/<id>/exits` | `[ExitView]`: every device an admin allowed as an exit, with `in_use_by` naming this device when it uses that exit. `access` is empty: every member may use an allowed exit |
| GET | `/api/networks/<id>/invites` | `[OpenInvite {id, expires_at_ms, uses_left, created_at_ms}]`, newest first: the invites in the network's ledger that are neither revoked, used up nor expired, whoever made them. `id` is the invite's public key in hex; `uses_left` is `null` for an unlimited invite |
| GET | `/api/networks/<id>/policy` | `PolicyInfo {toml, rules}`: the policy in force as the TOML `POST` takes, and its rules as `PolicyRuleInfo {sources: [SelectorInfo], destinations: [TargetInfo]}`. A `SelectorInfo` is `{kind, value, name}` with `kind` `any`, `device` (`value` the id, `name` its name when known) or `tag` (`value` the tag); a `TargetInfo` is `{selector, protocol, ports}`, `protocol` `any`, `tcp`, `udp` or `icmp` and `ports` like `["22", "8000-8080"]`, empty for every port. A network that never set a policy shows the allow-all one it runs |
| GET | `/api/networks/<id>/relays` | `[RelayView {name, address, node}]`: the relays advertised in the network |
| GET | `/api/server` | `ServerView {exit, relay}`: whether this device offers itself as an exit and a relay |
| GET | `/api/networks/<id>/request-code` | `JoinCode {code, command}`: the `kith://request/` code and `kith request <code>`, which a new device runs to ask an admin to let it in |
| GET | `/api/validate/name?net=&name=` | `NameCheck {name, problem, suggestion}`. `name` comes back lowercased with spaces as hyphens. `problem` is `empty`, `too_long`, `invalid_character`, `hyphen_at_edge` or `taken`; a taken name comes with the first free `name-2`, `name-3`, … as `suggestion`. `net` is a network id or name: the name is taken when a device there (this one included) has it. Without `net` it is a new network's name, taken when this device is already in a network of that name |

Every snapshot carries `boot` and `seq`; a stream entry applies only when its `seq` is above the snapshot's. `boot` and `seq` are read before the rest of the snapshot, so an entry that lands while it is read reaches the page on the stream rather than being lost.

`boot` names the daemon's run. The server follows the daemon's journal on one subscription and keeps its newest 4096 entries, the newest tick, and an hour of traffic per device (1800 samples), so the hour chart survives a daemon that keeps less. When the subscription comes back after the daemon was unreachable, the server checks that the daemon still holds the last entry it saw; if not, the daemon restarted and `boot` changes.

## Event stream

`GET /api/events`, server-sent events. It resumes from `Last-Event-ID: <boot>:<seq>`, or `?last=<boot>:<seq>` on the first connect; the header wins when both are sent. Without either, the stream starts with what happens next.

| Event | Id | Data |
|---|---|---|
| `entry` | `<boot>:<seq>` | a journal `Entry`, flattened to `{seq, at_ms, kind, ...fields}`. A rename is `device_renamed {network, node, old_name, name}` |
| `tick` | none, never replayed | `Tick {at_ms, traffic: [TrafficTick], links: [LinkTick]}`, every 2 s. A link carries its exact `rate` when this device is an admin of the network or is one of the link's two ends. Otherwise it carries `rate_band` (`idle` below 1 kB/s, `light` up to 1 MB/s, `busy` above) and `rate` is `null`. The boot snapshot's `last_tick` follows the same rule |
| `reset` | none | `Reset {boot, started_at_ms}`: the requested boot is not the current one, or its seq is ahead of the current run. Also sent to every open stream when the daemon restarts, right after `daemon_up`. The page refetches `/api/boot` |
| `gap` | none | `Gap {missed_from, missed_to}`: entries after the requested seq are no longer kept. The kept entries follow |
| `daemon_down`, `daemon_up` | none | `{}`. A stream opened while the daemon is unreachable starts with `daemon_down` |

On connect the stream sends, in order: `reset`, or `gap` and the kept entries after the requested seq; then `daemon_down` if the daemon is unreachable; then everything live. A page too slow to keep up is caught up from the kept entries, never skipped silently.

The band is presentation, not a privacy boundary: every member's daemon holds the exact rates that the others gossip, and `kith map` shows them. See `docs/security.md`.

There is no keep-alive comment: the tick is the heartbeat. A tick the daemon cannot answer within 2 s is skipped, so the page sees the values go stale.

## Actions

All `POST`, JSON in, JSON out. A `qr_svg` is `null` when the text is too long for a QR code. Changes return `Outcome {state, op}`. The daemon does not name the ledger op a change signed, so `op` is `null` and the page shows "Saved on this device" rather than a count.

- **Join requests.** A check code is six Crockford base32 characters (30 bits) from a BLAKE3 hash (context `kith 2026-11 join check code v1`) of the requesting device's public key, so the device and every admin compute the same code. It tells two requests apart by eye; it is not a secret. Approve and deny can be sent again safely: when the request is no longer pending, 409 says how it ended (`approved`, `denied` or `expired`) and when.
- **`based_on`.** `based_on` is the value the edit started from: a name's current value, a device's current name. When the thing now holds something else, the change is refused with 409 and `conflict.op` is what it holds now (`null` once deleted). `based_on: null` changes it whatever it holds.
- **Tier 3** (showing a phone's private config, leaving a network, changing what this device serves, importing a WireGuard config). The request stays open while the OS asks the person at this computer (Touch ID or the login password on macOS, polkit on Linux), one prompt at a time; the page shows `awaiting_os_auth` while it waits. It answers once confirmed, or 403 `os_auth_denied` with nothing signed or shown, also when nobody answers within 2 minutes. A computer that cannot ask, such as a Linux server without a polkit agent, always answers 403. The prompt guards against someone at an unlocked, signed-in browser tab; it does not stop the OS user who runs `kith-ui`, who can make the same change with `kith`.
- **Admin changes** asked by a member answer 403 `permission_denied`.

| Path | In | Out |
|---|---|---|
| `/api/networks` | `CreateNetworkIn {name, device_name}` | `Outcome` |
| `/api/join` | `JoinIn {invite_link, device_name}` | `Outcome` |
| `/api/join/request` | `RequestJoinIn {request_code, device_name}` | `JoinRequestState` |
| `/api/networks/<id>/joins/<node>/approve` | `ApproveIn {name}`; the device joins under `name`, or under the name it asked for when `name` is `null`, empty or absent | `Outcome`; 409 when already settled |
| `/api/networks/<id>/joins/<node>/deny` | `{}` | `Outcome`; 409 when already settled |
| `/api/networks/<id>/invites` | `InviteIn {expires_secs, uses}` | `Invite`; its `id` is the one the list shows |
| `/api/networks/<id>/invites/<id>/revoke` | `{}` | `Outcome`; signs a revocation, after which no device can join with the invite. 404 for an invite that is not open |
| `/api/networks/<id>/phones` | `PhoneIn {name, tags}` | `PhoneCreated`. The flow's one ledger write; this server keeps the phone's config until `kith-ui` stops |
| `/api/networks/<id>/phones/<node>/config` | `PhoneConfigIn {everything}` | `PhoneConfig`; tier 3. 404 for a phone made before `kith-ui` started; 400 for `everything` when this device offers no exit |
| `/api/networks/<id>/records` | `RecordIn {name, type, value, based_on}`, `name` without the network suffix | `Outcome`; 409 when `based_on` is stale |
| `/api/networks/<id>/records/<name>/delete` | `{}`; `<name>` is the full name the list shows, or the short one | `Outcome` |
| `/api/networks/<id>/devices/<node>/rename` | `RenameIn {name, based_on}`; `name` follows the join rules (not empty, at most 64 bytes) and every member sees it as the device's name, made a unique DNS label | `Outcome`; 409 when `based_on` is stale, with the current name in `conflict.op` |
| `/api/networks/<id>/devices/<node>/remove` | `RemoveIn {lost}`; with `lost`, the names an admin pointed at the device (by address or alias) go too | `Outcome`; 409 when another admin removed it first. This device leaves instead |
| `/api/networks/<id>/leave` | `{}` | `Outcome`; tier 3 |
| `/api/exit` | `ExitIn {network, node}`, both `null` for no exit | `Outcome` |
| `/api/networks/<id>/exits/<node>` | `GrantIn {allowed}` | `Outcome` |
| `/api/networks/<id>/policy` | `PolicyIn {toml}` | `Outcome`; 400 with the parser's line and column in `message` when the TOML does not parse |
| `/api/networks/<id>/relays` | `RelayIn {name, address, node}`; `node` is the device running the relay, `null` for this one. A relay of the same name is replaced | `Outcome` |
| `/api/networks/<id>/relays/<name>/remove` | `{}` | `Outcome`; 404 for a relay that is not advertised |
| `/api/server` | `ServerView {exit, relay}` | `Outcome`; tier 3 |
| `/api/wireguard/import` | `ImportIn {name, conf_text}`, a `wg-quick` config | `Imported {network}`; tier 3. 400 with the parser's message for a config that does not parse |
| `/api/wireguard/:net/use` | `ImportUseIn {active}` | 200; routes through an imported network (listed in `GET /api/status` as `imported`), or keeps it but stops. One that carries all internet traffic replaces the exit in use and any other such import. 404 for an unknown import |
| `/api/wireguard/:net/remove` | none | 200; removes the import and forgets its config; tier 3 |
| `/api/blocked/<id>/allow` | `AllowIn {preview}`. The id names the connection (`blocked:<network>:<from>:<to>:<protocol>:<port>`), so a second allow is harmless | `AllowPreview {rule_sentence, new_paths}` when previewing, such as `phone may reach nas on TCP 22`, with one path for the one rule; else `Outcome` |

Not in v1, so the page ships no button for them: force-handshake, pause with a deadline, generic member requests.

## Static

Only the routes of the design spec §4.1 serve the page: `/`, `/welcome`, `/this`, `/n/<net>` and its `devices`, `devices/<dev>`, `activity`, `names`, `exits`, `settings`. Hashed files under `/assets/` are served with immutable cache headers and brotli when the browser accepts it. Every other path is 404.

## Fixture mode

`kith-ui --fixture web/fixtures/office.json` simulates a daemon from a recorded network instead of talking to one; the boot payload's `boot` is the file's, so the page knows it is a simulation. Every time in the file moves so that it was recorded at startup. The recording goes through the same path as a daemon's journal:

- `boot` is the file's, and its journal is the history a page can resume from.
- Ticks every 2 s advance the recorded traffic counters step by step, with the recorded links.
- The recorded activity (peers online and offline, path and endpoint changes, drops, warnings, log lines) plays again, in a loop, as new entries numbered after the last one and stamped now. It keeps the recorded spacing 60 times faster, at least 0.5 s and at most 20 s apart. Joins, removals and exit changes are not replayed: they happen only when the page makes them.
- The recording never stops or restarts: no `daemon_down`, and `reset` only for a page that asks with another boot.

Every endpoint works. Reads answer from an in-memory copy of the file, and every change edits that copy and writes the journal entry a daemon would, so the page sees its own changes on the stream: approving a join adds the device (`device_joined`), removing one drops it (`device_removed`), renaming writes `device_renamed`, using an exit writes `exit_changed`, and adding a phone adds a device. Records, invites (real `kith://join/` links for a network no daemon serves), revocations, the policy (allowing a blocked connection adds its rule and clears the group), relays and server mode change what the reads return; an import checks the config as the daemon does and lists a made-up network in `imported`, which can be used, stopped and removed. Tier-3 changes do not prompt. Nothing reaches a daemon or the network, and changes last until `kith-ui` stops. `request-code` answers `kith://request/<network>-recorded`. `npm run fixture` regenerates the file.
