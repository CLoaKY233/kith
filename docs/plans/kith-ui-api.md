# kith-ui: HTTP API between the page and the `kith-ui` server

The page talks only to the `kith-ui` process on the same machine. `kith-ui` talks to the daemon over IPC (`kith-ipc`). JSON everywhere; field names are snake_case. Every type named here is generated into `kith-ui/web/src/lib/types.gen.ts` by `cargo xtask ui-types`, and a test in `kith-ui-core` fails while that file is stale.

This file follows `docs/plans/kith-ui-design.md` §2, which wins where they differ. The server is `kith-ui-core::router`, an axum `Router`.

## Session and guards

- `kith-ui` listens on `127.0.0.1:<port>` (default 41680, `--port` to change; a free port if taken).
- `kith ui` opens `http://127.0.0.1:<port>/#t=<token>`. The token is single-use, valid 2 minutes, compared in constant time and burned on use.
- The page POSTs `/api/session` with `SessionIn {token}`. The answer sets an `HttpOnly; SameSite=Strict` cookie holding a separate secret. The page then drops the fragment with `history.replaceState`.
- Every request must name this machine in `Host` (`127.0.0.1:<port>` or `localhost:<port>`). Every POST and `/api/events` must also carry a matching `Origin`. Under `--dev`, Vite's origin is allowed too.
- Every response carries the CSP `default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'`.
- Without the cookie, API calls get 401 and the page shell renders the signed-out page.

## Errors

Status 400, 401, 403, 404, 409, 422, 501 or 503 with `ApiErrorBody {code, message, conflict}`. `code` is an `ApiErrorCode`. A 409 fills `conflict` with `{resolved_by, resolution, at_ms, op}`: who won the race and how.

## Reads

| Method | Path | Body |
|---|---|---|
| GET | `/api/boot` | `Boot {boot, seq, server_time_ms, status, peers, traffic_tail, pending, versions, last_tick}`. The same object is inlined into `index.html` as `<script type="application/json" id="boot">` |
| GET | `/api/status` | `StatusView`: `Status` plus `operator`, `versions`, `paused_until`, `memberships` (role and admins per network), `host`, `exit_state` |
| GET | `/api/pending` | `Pending {joins: [JoinRequest], blocked: [BlockedGroup]}`. A join request carries `check_code`, `claimed {name, os}`, `seen {ip, key_fpr}` and `expires_at_ms` |
| GET | `/api/traffic` | `[TrafficSeries]` |
| GET | `/api/networks/<id>/peers` | `[PeerInfo]` |
| GET | `/api/networks/<id>/records` | `[RecordView]` |
| GET | `/api/networks/<id>/exits` | `[ExitView]` |
| GET | `/api/networks/<id>/invites` | `[Invite]` |
| GET | `/api/validate/name?net=&name=` | `NameCheck {name, problem, suggestion}`, from the same validator as the CLI |

Every snapshot carries `boot` and `seq`; a stream entry applies only when its `seq` is above the snapshot's.

## Event stream

`GET /api/events`, server-sent events. It resumes from `Last-Event-ID: <boot>:<seq>`, or `?last=<boot>:<seq>` on the first connect.

| Event | Id | Data |
|---|---|---|
| `entry` | `<boot>:<seq>` | a journal `Entry` |
| `tick` | none, never replayed | `Tick {at_ms, traffic: [TrafficTick], links: [LinkTick]}`, every 2 s. Members get `rate_band` instead of `rate` for other devices' links |
| `reset` | none | `Reset {boot, started_at_ms}`, when the requested boot is not the current one |
| `gap` | none | `Gap {missed_from, missed_to}`, when the requested seq has left the ring |
| `daemon_down`, `daemon_up` | none | none |

There is no keep-alive comment: the tick is the heartbeat.

## Actions

All `POST`, JSON in, JSON out. A `qr_svg` is `null` when the text is too long for a QR code. Changes that sign a ledger op return `Outcome {state, op}`; `state` is `accepted`, or `awaiting_os_auth` for tier-3 actions while the OS asks the person at the computer.

| Path | In | Out |
|---|---|---|
| `/api/networks` | `CreateNetworkIn {name, device_name}` | `Outcome` |
| `/api/join` | `JoinIn {invite_link, device_name}` | `Outcome` |
| `/api/join/request` | `RequestJoinIn {request_code, device_name}` | `JoinRequestState` |
| `/api/networks/<id>/joins/<node>/approve` | `{name}`; the server ignores `name` until the daemon can name on approval | `Outcome`; 409 if already resolved |
| `/api/networks/<id>/joins/<node>/deny` | `{}` | `Outcome`; 409 if already resolved |
| `/api/networks/<id>/invites` | `InviteIn {expires_secs, uses}` | `Invite` |
| `/api/networks/<id>/invites/<id>/revoke` | `{}` | `Outcome` |
| `/api/networks/<id>/phones` | `PhoneIn {name, tags}` | `PhoneCreated` |
| `/api/networks/<id>/phones/<node>/config` | `PhoneConfigIn {everything}` | `PhoneConfig`; tier 3 |
| `/api/networks/<id>/records` | `RecordIn {name, type, value, based_on}` | `Outcome` |
| `/api/networks/<id>/records/<name>/delete` | `{}` | `Outcome` |
| `/api/networks/<id>/devices/<node>/rename` | `RenameIn {name, based_on}` | `Outcome` |
| `/api/networks/<id>/devices/<node>/remove` | `RemoveIn {lost}` | `Outcome` |
| `/api/networks/<id>/leave` | `{}` | `Outcome`; tier 3 |
| `/api/exit` | `ExitIn {network, node}`, both `null` for no exit | `Outcome` |
| `/api/networks/<id>/exits/<node>` | `GrantIn {allowed}` | `Outcome` |
| `/api/blocked/<id>/allow` | `AllowIn {preview}` | `AllowPreview {rule_sentence, new_paths}` when previewing, else `Outcome` |

Not in v1, so the page ships no button for them: force-handshake, pause with a deadline, generic member requests.

## Static

Only the routes of the design spec §4.1 serve the page: `/`, `/welcome`, `/this`, `/n/<net>` and its `devices`, `devices/<dev>`, `activity`, `names`, `exits`, `settings`. Hashed files under `/assets/` are served with immutable cache headers and brotli when the browser accepts it. Every other path is 404.

## Fixture mode

`kith-ui --fixture web/fixtures/office.json` serves a recorded network instead of talking to the daemon: the snapshot from the file, then its journal and traffic replayed on the stream. Changes answer 501 there. `npm run fixture` regenerates the file.
