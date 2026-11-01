# kith-ui: HTTP API between the page and the `kith-ui` server

The page talks only to the `kith-ui` process on the same machine. `kith-ui` talks to the daemon over IPC (`kith-ipc`). JSON everywhere; field names are snake_case and match the `kith-ipc` types where one exists.

## Session

- `kith-ui` listens on `127.0.0.1:<port>` (default 41680, `--port` to change; a free port if taken).
- At start it makes a random 32-byte token and opens `http://127.0.0.1:<port>/?token=<hex>`.
- `GET /?token=<hex>` with the right token sets a cookie `kith_ui=<hex>; HttpOnly; SameSite=Strict; Path=/` and redirects to `/`. Every other request without the cookie gets 401 and a page saying to run `kith ui`.
- Every `POST` must also carry `Origin` (or `Referer`) equal to `http://127.0.0.1:<port>`; otherwise 403.

## Reads

| Method | Path | Body |
|---|---|---|
| GET | `/api/status` | `Status`, plus `"admin": {"<network id>": bool}` for whether this device may change each network |
| GET | `/api/networks/<id>/peers` | `[PeerInfo]` |
| GET | `/api/networks/<id>/records` | `[RecordInfo]` as `{name, type, value, device}` |
| GET | `/api/traffic` | `[TrafficSeries]` |
| GET | `/api/events?since=<seq>` | Server-sent events, one per journal `Entry`: `id: <seq>`, `event: <kind in snake_case>`, `data: <Entry as JSON>`. `Last-Event-ID` resumes. A comment line every 15 s keeps it open. |

Ids (network, node) are lowercase hex. Times are `at_ms` (Unix milliseconds).

## Actions

All `POST`, JSON in, JSON out. Errors: status 400/403/404/409/500 with `{"error": "<plain words>"}`.

| Path | In | Out |
|---|---|---|
| `/api/networks/<id>/invites` | `{expires_secs, uses}` | `{link, qr_svg}` |
| `/api/networks/<id>/phones` | `{name, tags: []}` | `{device: {config, qr_svg}, everything: {config, qr_svg}}` |
| `/api/networks/<id>/devices/<node>/remove` | `{}` | `{}` |
| `/api/networks/<id>/records` | `{name, type, value}` | `{}` |
| `/api/networks/<id>/records/<name>/delete` | `{}` | `{}` |
| `/api/exit` | `{network, node}` or `{network: null}` | `{}` |
| `/api/networks/<id>/exits/<node>` | `{allowed: bool}` | `{}` |

## Static

`GET /` serves the app; `GET /assets/<file>` serves files compiled into the binary. No other paths.
