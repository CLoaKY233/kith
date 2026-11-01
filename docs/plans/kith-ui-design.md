# kith UI: the design and build spec

This is the single definitive spec. Where it conflicts with an earlier proposal or critique, this spec wins. Engineers build from this document alone.

---

## 0. Decisions the brief got wrong, and the rulings

1. **Astro is the wrong tool.** Every pixel on this page is live, so islands add nothing.
2. **Native Rust toolkits are not used.** That covers gpui, egui, Iced, Slint and Dioxus. Three facts decide it:
   - The product needs a browser page anyway: headless servers, `ssh -L`, the read-only status page.
   - Their accessibility (live regions, grids, focus-trapped dialogs, IME) is not good enough.
   - Using one would mean writing the UI twice.
3. **The map is not the home screen.** People open kith for about 8 seconds to ask "is it working, does anything need me?" A sentence answers that; a graph doesn't. The map is a view on Devices.
4. **Colour is not a five-state rainbow.** Healthy has no hue. Relayed is not a fault, so it gets no colour. Only two signal hues exist: amber (a person is needed) and red (broken, refused or destructive). There is one accent, used for the primary action and for focus.
5. **"One click to allow just this" is never one click.** It always shows the rule first, then signs.
6. **Join approval by name alone is unsafe.** Every join request carries a check code, shown on both devices.
7. **Members never see disabled buttons.** They get an "Ask an admin" path in the same place.
8. **A web tab alone can't keep the promise that a request reaches an admin within a second.** A Tauri 2 tray shell is milestone M2. It is required, not optional polish.
9. **"Control room" is something you can opt into**, not the landing page.

---

## 1. Stack and project layout

### 1.1 Choices (final)

| Layer | Choice |
|---|---|
| Server | **axum 0.8** on the workspace's tokio and hyper. SSE uses `axum::response::sse` |
| Server structure | **`kith-ui-core`** library returns an `axum::Router`. `kith-ui` binary is about 40 lines. Tauri (M2) mounts the same router on loopback |
| Page | **Svelte 5 (runes), plain SPA, Vite 6, TypeScript strict.** No SvelteKit |
| Primitives | **Bits UI 1.x.** Components are copied into the repo shadcn-svelte style and restyled. Toasts use **svelte-sonner**. No second headless library |
| Styling | **Tailwind CSS v4.** `@theme` maps only to our semantic tokens. An ESLint rule bans raw palette classes (`text-blue-500`, `bg-gray-*`) |
| Detail charts | **uPlot** |
| Sparklines | **Hand-written Canvas 2D:** one shared painter per list, no library |
| Map | **Hand-built SVG**, with a deterministic ring layout. **No d3-force, no worker** |
| Icons | **Lucide subset** at 16 px, 1.5 stroke, inlined as a sprite, plus 4 custom glyphs: `path-direct`, `path-relayed`, `path-stealth`, `seal` |
| Fonts | **Inter 4 variable** and **JetBrains Mono variable**, Latin subset, woff2, embedded (about 90 KB) |
| QR | Server side, with the `qrcode` crate returning `qr_svg`. No JS QR library |
| Transport | **HTTP JSON for reads and writes, one SSE stream for everything live.** No WebSocket |
| Embedding | `rust-embed`, brotli quality 11 at build time, immutable cache headers for hashed assets |
| Types | **`ts-rs`** exports `kith-ipc` types to `web/src/lib/types.gen.ts`. CI fails on diff |
| Shell (M2) | **Tauri 2:** native tray menu (not a webview popover), notifications, autostart, `kith://` deep links. The webview loads the same router over HTTP. No `invoke` for data. The webview is destroyed when the window closes |

### 1.2 Repository layout

```
crates/management/kith-ui-core/
  src/lib.rs            router(cfg) -> Router
  src/session.rs        token→cookie, Host/Origin guard (tower layer), CSP headers
  src/ipc.rs            the ONLY daemon client; snapshot + journal subscription
  src/stream.rs         SSE fan-out: boot:seq ids, reset/gap, tick every 2 s
  src/traffic.rs        per-peer 1 h ring buffers (1800 samples)
  src/api/*.rs          status, peers, traffic, records, exits, invites, phones,
                        joins, blocked, devices (rename/remove), validate
  src/osauth.rs         tier-3 gate: LocalAuthentication (macOS), polkit (Linux)
  src/assets.rs         rust-embed handler, SPA route allowlist, boot JSON inlining
crates/management/kith-ui/        main.rs: args, port, open browser (`opener`)
crates/management/kith-ui/web/    the Svelte app (see §1.3)
crates/management/kith-ui/fallback.html
apps/kith-app/                    Tauri 2 shell (M2)
```

### 1.3 Web layout

```
web/
  index.html  vite.config.ts  tailwind.css  eslint.config.js
  src/
    main.ts  App.svelte  router.ts (~60 lines, pushState, no library)
    lib/
      tokens.css      tokens.ts (canvas mirror, read via getComputedStyle on theme change)
      types.gen.ts    (generated)   types.ts (UI-only types)
      api.ts          fetch wrapper → ApiError{code,status,message}
      live/
        store.svelte.ts   the single store
        stream.ts         EventSource, reducers, rAF commit
        reducers.ts       switch(kind) with exhaustiveness check
        timing.ts         ALL time constants (§8.6)
      copy.ts         ALL user-facing strings (§12), linted
      format.ts       bytes, rates, RTT, relative time, check codes
      announce.ts     polite/assertive announcer queue
      ui/             Button, IconButton, Dialog, AlertDialog, Sheet, Popover, Tooltip,
                      DropdownMenu, Command, Tabs, Segmented, Switch, Input, Field,
                      Toast, Badge, Kbd, CopyField, Spinner, StatusDot, RoleChip
      icons/          sprite.svg + Icon.svelte
      charts/         Spark.svelte + sparkPainter.ts, Series.svelte (uPlot), resample.ts
      map/            Map.svelte, layout.ts (pure), MapTable.svelte
      kith/           DeviceRow, PathBadge, Rate, Rtt, NeedsYouCard, EventRow,
                      DeviceDetail, ExitPicker, QrPanel, CheckCode, Headline,
                      IdentityBlock, ConnectionBanner, AskAdminSheet
    routes/
      Welcome.svelte  Overview.svelte  Devices.svelte  Device.svelte
      Activity.svelte Names.svelte     Exits.svelte    Settings.svelte  ThisDevice.svelte
      Gate.svelte     (daemon down / not in group / version skew / 401)
    sheets/           AddDevice.svelte  AddPhone.svelte  Invite.svelte  JoinNetwork.svelte
                      CreateNetwork.svelte  AllowBlocked.svelte  RemoveDevice.svelte
  tests/              playwright/*.spec.ts, fixtures/office.json
```

### 1.4 Build and release

- **CI job `ui`:**
  1. `pnpm i --frozen-lockfile && pnpm build`.
  2. `size-limit` fails the job above **150 KB brotli** for the full first load, fonts included. The target is 110 KB.
  3. Playwright plus axe-core.
- **Release tags commit `web/dist/`.** The crate sets `include = ["web/dist/**"]`. `cargo install` never needs Node.
- **`build.rs`:** if `web/dist/index.html` is missing, embed `fallback.html` with a `cargo:warning=`. The build never fails.
- **Dev:** `pnpm dev` proxies `/api` to `kith-ui --dev`. Dev mode allows the Vite origin and serves no embedded assets.
- **Fixture mode:** `kith-ui --fixture fixtures/office.json` replays a recorded journal through a fake IPC.

---

## 2. Server contract: changes to `docs/plans/kith-ui-api.md`

Workstream A owns these. Each has a v1 fallback marked **[fallback]**.

1. **Login:**
   - `kith ui` opens `http://127.0.0.1:<port>/#t=<token>`.
   - The page POSTs `/api/session` with the token. The server answers with an `HttpOnly; SameSite=Strict` cookie holding a **separate** secret.
   - The page then calls `history.replaceState` to drop the fragment.
   - The token is single-use, valid 2 min, compared in constant time (`subtle`), and burned on use.
2. **Every request checks `Host`** against `127.0.0.1:<port>` and `localhost:<port>`. Every POST and `/api/events` also checks `Origin`.
3. **CSP:** `default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'`.
4. **SPA routes are an explicit allowlist** (§4). Everything else returns 404.
5. **Boot snapshot.** `index.html` inlines `<script type="application/json" id="boot">` with `{boot, seq, server_time_ms, status, peers, traffic_tail(150), pending, versions}`.
6. **SSE `/api/events`:**
   - `id: <boot>:<seq>`.
   - `event: entry` carries a journal `Entry`.
   - `event: tick` every 2 s: `{at_ms, traffic:[{network,node,rx,tx}], links:[{from,to,path,relay?,rtt_ms?,loss?,rate_band|rate}]}`. A tick has no id and is never replayed.
   - `event: reset {boot, started_at_ms}` is sent on a boot mismatch.
   - `event: gap {missed_from, missed_to}` is sent when the requested seq has left the ring.
   - `event: daemon_down` / `daemon_up`.
   - There is no keep-alive comment. The tick is the heartbeat.
7. **Every snapshot response carries `boot` and `seq`.** A stream entry applies only when `seq > snapshot.seq`.
8. **`GET /api/pending`** returns join requests and blocked-connection groups.
9. **`/api/status` adds** `operator: bool`, `versions {ui, daemon, compatible}`, `admin` per network and `paused_until?`.
10. **Join requests carry:**
    - `check_code`: 6 Crockford base32 characters derived from a hash of the requester's public key, displayed `K7Q·4XM`.
    - `claimed {name, os}`.
    - `seen {ip, key_fpr}`.
    - `expires_at_ms`.

    Approve and deny are idempotent. **409** returns `{resolved_by, resolution, at_ms}`. The requester's daemon receives the outcome.
11. **Mutating POSTs accept `based_on`.** 409 returns the conflicting op and its author.
12. **Propagation acks:** `entry kind: op_seen {op, seen_by, of}`. **[fallback]** Without them the UI shows "Saved on this device" and never shows a count.
13. **`POST /api/blocked/:id/allow`** has a `preview` mode that returns `{rule_sentence, new_paths}`.
14. **`GET /api/validate/name?net=&name=`** runs the same Rust validator as the CLI and checks whether the name is taken.
15. **`PathChanged` entries carry `from` and `to`.** Link reports carry RTT and loss. Members receive `rate_band` (`idle|light|busy`), never exact rates of other devices.
16. **Tier-3 actions** block server side on `osauth.rs` before signing. The page sees `state: "awaiting_os_auth"`.
17. **Out of v1:** force-handshake, pause-with-deadline, generic member requests. The UI does **not** ship buttons for them until the endpoint exists.

---

## 3. Design tokens (`tokens.css`, exact)

### 3.1 Colour, light (`:root`)

| Token | Value | Use |
|---|---|---|
| `--bg` | `#F9FAFB` | page |
| `--surface` | `#FFFFFF` | panels, sheets, the map well, the Needs-you list |
| `--sunken` | `#F3F4F6` | input fill, chart wells, code |
| `--line` | `#E4E6EA` | hairlines |
| `--line-strong` | `#8A8F98` | input and toggle borders (≥3:1, WCAG 1.4.11) |
| `--ink-1` | `#17191C` | primary text, online marks |
| `--ink-2` | `#5B6069` | secondary |
| `--ink-3` | `#6F747C` | meta, offline text (≥4.5:1 on surface) |
| `--ink-4` | `#B4B8BE` | decorative only, always paired with text |
| `--accent` | `#2F62D0` | the one primary button, focus ring, selected chart series |
| `--accent-fg` | `#FFFFFF` | text on accent |
| `--attn` | `#C87A12` | needs-a-person marks |
| `--attn-text` | `#9A5A00` | needs-a-person text |
| `--attn-wash` | `#FDF5E8` | needs-a-person wash |
| `--danger` | `#C8322B` | broken, refused, destructive |
| `--danger-wash` | `#FDF0EF` | destructive wash |
| `--changed` | `rgb(47 98 208 / .08)` | the 4 s change tint |
| `--scrim` | `rgb(17 19 22 / .45)` | behind dialogs |

### 3.2 Colour, dark

Dark tokens apply under `@media (prefers-color-scheme: dark){:root:not([data-theme=light])}` and under `[data-theme=dark]`.

| Token | Value |
|---|---|
| `--bg` | `#111316` |
| `--surface` | `#181A1E` |
| `--sunken` | `#0D0F11` |
| `--line` | `#2A2D32` |
| `--line-strong` | `#6B7079` |
| `--ink-1` | `#EDEEF0` |
| `--ink-2` | `#AEB2B9` |
| `--ink-3` | `#8E929A` |
| `--ink-4` | `#4A4E55` |
| `--accent` | `#6A95EC` |
| `--accent-fg` | `#0D0F11` |
| `--attn` | `#E2A84A` |
| `--attn-text` | `#E2A84A` |
| `--attn-wash` | `#2A2112` |
| `--danger` | `#EC6B5F` |
| `--danger-wash` | `#2C1715` |
| `--changed` | `rgb(106 149 236 / .10)` |

Floating layers in dark mode get `box-shadow: inset 0 1px 0 rgb(255 255 255 / .08)` instead of a shadow.

`@media (forced-colors: active)`: status shapes stay visible and chart lines use `CanvasText`.

### 3.3 Type

- **Body:** `font-family: "Inter", ui-sans-serif, system-ui, sans-serif; font-feature-settings: "cv05","cv08","ss03"; font-variant-numeric: tabular-nums slashed-zero;`
- **Mono:** `"JetBrains Mono", ui-monospace, Menlo, monospace`. Use it only for addresses, keys, endpoints, ports, commands and log lines.
- Fonts load with `font-display: block`. They are local, so there is no flash.

| Token | px / line height | Weight | Tracking | Use |
|---|---|---|---|---|
| `--t-display` | 28/34 | 600 | -0.02em | the Overview headline, one per screen |
| `--t-title` | 20/28 | 600 | -0.014em | screen title |
| `--t-heading` | 15/22 | 600 | -0.006em | section header |
| `--t-body` | 14/20 (16/24 under `pointer:coarse`) | 400 | — | default text |
| `--t-small` | 12.5/18 | 450 | — | metadata |
| `--t-micro` | 11/14 | 550 | +0.02em | chart axes, column headers in `all-small-caps` |
| `--t-mono` | 13/20 | 400 | — | machine strings |

- Inputs are always 16 px (stops iOS zooming on focus).
- Weights in use: 400 and 600. 450 and 550 are optical tweaks for small sizes only.
- Units sit after a thin space in `--ink-3`: `12.4 MB/s`.
- Prose measure is at most 68ch.

### 3.4 Space, shape, layers

- **Spacing:** `--s-1 4`, `--s-2 8`, `--s-3 12`, `--s-4 16`, `--s-5 24`, `--s-6 32`, `--s-7 48`, `--s-8 64`. No other values.
  - Inside a component: 4–12.
  - Between components: 16–24.
  - Between sections: 48 (space separates sections, not lines).
- **Radii:** `--r-1 4` (badges), `--r-2 6` (controls), `--r-3 10` (panels, sheets, the map well), `--r-full`.
- **Hairlines:** 1 px `--line`, 0.5 px at `min-resolution: 2dppx`.
- **Shadows:**
  - `--shadow-pop`: `0 1px 2px rgb(0 0 0/.06), 0 8px 24px rgb(0 0 0/.10)` for menus, popovers and the drawer.
  - `--shadow-modal`: `0 24px 64px rgb(0 0 0/.18)` for dialogs.
- **Scrim:** plain, no `backdrop-filter`, because WebKitGTK is slow with it.
- **Row height:** 44 px everywhere. Under `pointer:fine`, Settings → Display offers Compact at 36 px. This is the only density option.
- **Z-order:** content 0, sticky header 10, drawer 20, popover 30, dialog 40, toast 50.

### 3.5 Motion (in `timing.ts` and as CSS variables)

| Use | Duration | Easing |
|---|---|---|
| hover / pressed colour | 80 ms | linear |
| popover in | 140 ms | `cubic-bezier(.2,.8,.2,1)` |
| popover out | 100 ms | `cubic-bezier(.2,.8,.2,1)` |
| drawer / sheet | 240 ms | `cubic-bezier(.32,.72,0,1)` |
| dialog | 180 ms, opacity plus scale .97→1 | — |
| row insert / remove | 160 ms (height plus fade) | — |
| change tint | 4 s fade from `--changed` | — |

- Only `transform` and `opacity` animate (the row-height collapse uses `animate:flip` or grid rows).
- **Never moves:**
  - numbers (only their text changes, with no counting animation);
  - a row under the pointer or holding focus;
  - layout when a toast or badge appears;
  - the primary button's position;
  - map nodes, unless the node set changes.
- **Never pulses:** no pulsing dots, no crawling dashes, no looping animation of any kind.
- **`prefers-reduced-motion`:** all transitions become 0 ms. The change tint still fades (opacity is not motion). Charts step once per sample instead of sliding.

---

## 4. Navigation, URLs, frame

### 4.1 Routes (the server allowlist)

```
/                      → /n/<last network> | /welcome
/welcome
/n/:net                Overview
/n/:net/devices        ?view=list|map  &q=  &filter=online|offline|relayed|phones|exits
/n/:net/devices/:dev   ?range=1m|15m|1h  &at=<ms> (parked cursor)
/n/:net/activity       ?kind=changes|joins|paths|blocked|logs  &device=  &q=
/n/:net/names
/n/:net/exits
/n/:net/settings
/this                  this device across networks: keys, role per network, daemon log, version
```

- Overlays use query parameters, so Back closes them:
  - `?add=device|phone|invite`
  - `?need=<id>` opens a Needs-you sheet
  - `?remove=<dev>`
- Back never undoes a signed action.
- Networks are addressed by name, falling back to a short id when two names collide.
- The browser title is `<this device> · <network> — kith`, prefixed `(n) ` when unseen changes exist.

### 4.2 Desktop frame (≥1280 px; the sidebar also shows from 1024 px)

```
┌──────────────────────────┬──────────────────────────────────────────────────┐
│ IdentityBlock            │ Header 56px: Title        [Exit · vps] [◔ 2] ⌘K  │
│  laptop                  │──────────────────────────────────────────────────│
│  100.64.0.3 · home ▾     │                                                  │
│  [Admin]                 │  content: max 1120px, 32px side padding          │
│                          │                                                  │
│  Overview                │                                                  │
│  Devices          12     │                                                  │
│  Activity                │                                                  │
│                          │                                                  │
│  Names                   │                                                  │
│  Exits                   │                                                  │
│                          │                                                  │
│  + Add device            │                                                  │
│ ─────────────            │                                                  │
│  Settings                │                                                  │
└──────────────────────────┴──────────────────────────────────────────────────┘
       232 px
```

**Sidebar** (`--surface`, 1 px right hairline):

- **IdentityBlock:**
  - Device name at 15/600.
  - Mono address · network name with `▾`. The `▾` shows only when more than one network exists; clicking it opens the network switcher.
  - `RoleChip`.
- **Nav groups:**
  - Observing: Overview, Devices, Activity.
  - A 24 px gap.
  - Shaping: Names, Exits.
  - A 24 px gap.
  - `+ Add device`, a text-styled nav item in `--ink-1`.
  - Settings, pinned to the bottom.
- **Nav item:** 32 px tall, a 16 px icon, the label at 14/400, and a count in `--ink-3`. The active item is `--ink-1` 600 on `--sunken`. No coloured bar.

**Header** (sticky, `--bg`, hairline bottom):

- Screen title at `--t-title`.
- On the right, in this order:
  - **ExitChip**, shown only when an exit is active.
  - **NeedsYouButton**: `--attn` dot plus count, shown only when the count is above 0 for *this* person. It opens a popover listing `NeedsYouCard`s.
  - **ConnectionIndicator**: nothing when live.
  - ⌘K button `Search ⌘K` (16 px icon plus a Kbd).

**Content:** `--bg`. Sections are plain, not cards.

### 4.3 Tablet (640–1279 px)

- **640–1023 px:** the sidebar collapses to a **64 px rail** with icons and tooltips. IdentityBlock becomes a 32 px circle with the device's initial; clicking it opens a popover holding the full block and the switcher.
- **640–1279 px:** device detail opens as a 440 px **overlay drawer** from the right with `--shadow-pop`. The list behind it does not move.
- **≥1280 px:** device detail is a **split panel** of 440 px and the list stays interactive.

### 4.4 Mobile (<640 px)

- **Top bar, 52 px:** network name as a button `home ▾`, which opens the **Network sheet**, then the RoleChip, then ExitChip if active.
- **Bottom tab bar:** 56 px plus `env(safe-area-inset-bottom)`, four tabs, each with icon and label: **Overview · Devices · Activity · Network**.
  - The Overview tab carries the amber Needs-you count, announced as "Overview, 2 need you".
  - The **Network tab** opens the same Network sheet (it holds no route): switch network, Names, Exits, Add device, Settings, This device.
- Detail is a **full page** with a back arrow, never a bottom sheet with detents. Back restores the list's scroll position.
- Sheets for flows are full-height pages with the primary action pinned at the bottom.
- 16 px side gutter, no horizontal scroll anywhere.

Breakpoints are written in rem: `40rem`, `64rem`, `80rem`. At 200% zoom nothing clips, and at 400% the layout becomes the mobile one.

---

## 5. Component inventory (states are mandatory)

All components live in `lib/ui` or `lib/kith`. All strings come from `copy.ts`.

**Button**
- Variants:
  - `primary`: `--accent` fill.
  - `secondary`: `--surface`, 1 px `--line-strong` border.
  - `ghost`: transparent.
  - `danger-quiet`: secondary with `--danger` text.
  - `danger`: filled `--danger`, used only inside the remove dialog.
- Heights 32 px (fine pointer) and 40 px (coarse). Padding 12 px. Label 14/600.
- States: rest, hover (background → `--sunken`, no movement), active, `:focus-visible` (2 px `--accent` outline, 2 px offset), `aria-disabled`, pending.
- **Pending:** the spinner appears only after 150 ms. It is a 12 px spinner replacing the label at the same min-width, kept for at least 300 ms once shown.
- **At most one primary per view.**
- Disabled is only used for: a stale connection (with inline reason text) or a type-to-confirm button. **Never** for form validation.

**IconButton:** 32×32 (44×44 on touch). Requires `aria-label` and a Tooltip with the same text.

**StatusDot:** 8 px.
- Online: filled `--ink-1`.
- Offline: 1.5 px ring in `--ink-3`.
- Stale (last known): ring in `--ink-1`.
- Problem: filled `--danger`.
- Needs you: filled `--attn`.
- Always paired with a word. Accessible name e.g. "online".

**PathBadge:** text plus glyph, no pill fill.
- `path-direct` + `direct`.
- `path-relayed` + `via vps`.
- `path-stealth` + `stealth via vps`.
- Hover or focus opens a HoverCard after 400 ms showing endpoint (mono), MTU and `path_reason` mapped to a sentence. On touch it does nothing; the row opens the detail.
- Flapping (more than 3 changes in 60 s) adds the badge `flapping` in `--ink-3`.
- A style change waits for 5 s of stability before applying.

**Rate:** a fixed `7ch` slot, right aligned.
- Bytes, SI (1000), 3 significant figures: `824 kB/s`, `1.24 MB/s`.
- Below 1 kB/s it reads `idle` in `--ink-3`.
- Stale shows `—`.
- Members see `idle / light / busy` for other devices' links.
- Decision: rates use bytes so every number on the page shares one unit family.

**Rtt:** a `6ch` slot showing `12 ms`, `142 ms` or `—`.

**RelativeTime:** `now`, `12 s ago`, `4 min ago`, then `14:02` after 1 h, then `Tue 14:02`. Wrapped in `<time datetime title=absolute>`. Updates every 1 s inside a fixed slot. Computed with a server clock offset taken from boot.

**DeviceRow:** the core component, 44 px.

| Width | Columns |
|---|---|
| Desktop grid | `20px dot · 1fr name + mono address(--ink-3, 12.5) · 150px PathBadge · 64px Rtt · 120px Spark · 84px Rate · 96px last seen · 32px ⋯` |
| Tablet | drops address and last seen |
| Mobile | two lines, 64 px: line 1 is dot, name, PathBadge on the right; line 2 is `100.64.0.7 · 34 ms · ↓ 1.2 MB/s` in `--ink-2` |

- This device is pinned first with the tag `you`.
- **States:**
  - Hover: `--sunken`.
  - Focus: inset ring.
  - Selected: `--sunken` plus a 2 px `--ink-1` leading bar.
  - Offline: text in `--ink-3`, hollow dot, `seen 3 h ago`.
  - Never seen: `not connected yet`, plus `added by laptop 2 min ago`.
  - Waiting phone: a dashed 1 px outline on the dot and `waiting for first connection`.
  - Changed: 4 s tint.
  - Needs you: a 2 px `--attn` leading bar plus `--attn-wash`.
  - Removing: `Removing…` in place, then "Removed" for 1.5 s, then a 160 ms collapse.
- **Accessibility:** the list is a `role="grid"` with roving tabindex.
  - ↑/↓ move between rows, Enter opens the detail, `.` opens the ⋯ menu, typeahead jumps by name.
  - The accessible name is computed on focus: "nas, online, direct, 3 milliseconds, receiving 1.2 megabytes per second".
  - Number cells are `aria-live=off`.

**Spark:**
- Size 120×24 on desktop; on mobile it fills the row width.
- One shared canvas painter per list, redrawn in the tick's rAF.
- Draws the rx area at 12% `--ink-1` with a 1.25 px `--ink-1` line. tx is not drawn; it appears as text in Rate.
- Own y-scale per row. Rescale only when the new max is above 1.25× or below 0.5× the current scale, easing over 300 ms.
- Breaks the line at gaps over 5 s, drops negative deltas, and never draws a line across a gap.
- Hover shows a crosshair plus a tooltip `14:02:16 · ↓ 2.1 MB/s · peak 4.0 MB/s`.
- Empty: a dashed baseline. Offline: frozen in `--ink-4`.

**Series** (uPlot detail chart):
- Full width, 200 px tall (160 on mobile). Mirrored: received above the baseline, sent below in `--ink-2`.
- Up to 3 nice ticks on the right. Horizontal hairlines at `--line`. Relative x labels `−60m … now`.
- Hover readout pinned at the top: `14:32:08 · ↓ 12.4 MB/s ↑ 860 kB/s · direct`.
- On touch: press and drag; the readout lingers 2 s after release.
- **Event pins:** 4 px ticks on the x-axis for this device's journal entries. Hover shows the entry sentence; click opens it in Activity.
- **Relayed band:** a 4% `--ink-1` band behind periods where the path was relayed, labelled `relayed` when wider than 60 px.
- Max-bucket resampling to W/2 points.
- Range control `1m · 15m · 1h` (Segmented), stored in the URL and shared by every chart on the screen.
- `role="img"` with a generated summary, plus a `Show data` disclosure that renders a table of 1-minute buckets.

**Segmented:** a Bits ToggleGroup in single mode. 32 px tall, `--sunken` track, the selected segment on `--surface` with a hairline. A disabled segment carries an inline reason under the control.

**Input / Field:**
- Label above the field, never a placeholder used as the label. Helper text in `--ink-3`.
- Error in `--danger` under the field, wired with `aria-describedby` and `aria-invalid`.
- Validate on blur, then on every keystroke once the field is in error.
- Name inputs set `autocapitalize=none autocorrect=off spellcheck=false enterkeyhint=done`.

**CopyField:** a mono value on `--sunken` plus `Copy`, which becomes `Copied` (check icon) for 1.5 s.

**Dialog / AlertDialog:**
- Bits primitives, max width 440, `--r-3`, `--shadow-modal`, plain scrim.
- Esc closes. Focus returns to the element that opened it.
- No modal ever opens from another modal.

**Sheet:**
- Desktop: a 480 px right-side sheet for flows.
- Mobile: full screen.
- A sheet that is part of a flow stays open until the outcome or the user cancels.

**Drawer:** the device detail container (§4.3).

**Popover / Tooltip / DropdownMenu / Command:** Bits primitives with `--shadow-pop`. Tooltips have a 400 ms delay, never carry essential information, and are skipped on touch.

**Toast** (svelte-sonner):
- Bottom-right on desktop, top on mobile. At most 3. 5 s, paused on hover.
- Only for the user's own actions. Network events never produce toasts.
- Optional `Undo`.
- Error toasts stay until dismissed.

**RoleChip:** 20 px, text only.
- `Admin`: `--ink-1` on `--sunken`.
- `Member`: outline.
- `View only`: outline, `--ink-3`.
- `Removed`: `--danger` text on `--danger-wash`.
- Click opens a popover:
  - Admin: "This device signs changes to home with its own key. Every other device checks the signature." · `Key k7f3·9a21·… Copy` · `Made admin by desk · 12 Oct`.
  - Member: "This device can see home. Changes need an admin: desk or nas."
- The chip updates **before** the content when the network switches.

**ExitChip:** `Exit · vps` with the globe icon, opens the ExitPicker popover.
- Exit offline: `--danger` text, `vps offline · traffic blocked` or `vps offline · going direct`, whichever the daemon reports.

**NeedsYouCard:** one component used in four places: the Overview strip, the header popover, the AddDevice sheet and `?need=` (§7.5).
- `--surface`, hairline, a 2 px `--attn` leading bar.
- Kinds: `join`, `blocked`, `health`. Health comes from `Status.problems` and `warnings`.

**EventRow:** `56px time · 16px glyph (only for blocked, warning, seal) · sentence · actor`.
- Actor is `signed by laptop` (followed by the seal glyph) or `reported by phone`.
- Grouped rows are collapsible.

**CheckCode:** mono 20/600, letter-spacing .08em, formatted `K7Q·4XM`, on `--sunken`, `--r-2`. Screen readers hear it spelled out ("K 7 Q, 4 X M").

**QrPanel:**
- 240 px QR on a forced white plate with a 24 px quiet zone, in both themes.
- Hidden behind `Show QR code`. Auto-hides on the first handshake or after 5 min.

**Headline:** `--t-display` sentence with an optional inline action (§6.2).

**ConnectionBanner:** a full-width bar under the header. Neutral for reconnecting, `--attn-wash` for daemon down.

**StaleLabel:** `as of 14:02` in the header, in `--ink-3`.

**Icons** (Lucide unless marked custom):

| Meaning | Icon |
|---|---|
| Overview | `activity` |
| Devices | `monitor-smartphone` |
| Activity | `list` |
| Names | `at-sign` |
| Exits | `globe` |
| Add | `plus` |
| Settings | `settings-2` |
| Needs you | `inbox` |
| Search | `search` |
| Copy | `copy` / `check` |
| More | `ellipsis` |
| Blocked | `ban` |
| Warning | `triangle-alert` |
| Error | `circle-x` |
| QR | `qr-code` |
| Invite | `ticket` |
| Device kinds | `laptop`, `monitor`, `smartphone`, `server`, `cpu` (unknown) |
| Admin | `key-round` |
| Relay | `radio-tower` |
| Paths, signed | custom: `path-direct`, `path-relayed`, `path-stealth`, `seal` |
| Brand | the kith mark |

- No icons beside worded buttons. Never an icon alone for state. Never a brand or OS logo.

---

## 6. Screens

### 6.1 Welcome (`/welcome`, no networks)

No app frame. Centred column, max 640 px.

- `kith` at 20/600.
- "A private network between your devices. No server. No account." in `--ink-2`.
- Two large card buttons, side by side on desktop and stacked on mobile:
  - **Start a network**, with the subtext "This device becomes its first device and its admin."
  - **Join a network**, with the subtext "You have an invite link, or someone will approve you."
- Footer line: `This device: studio-mac · macOS` plus `Rename`.

### 6.2 Overview (`/n/:net`)

Top to bottom, with 48 px between sections.

**1. Headline.** The first condition that matches wins.

| Priority | Condition | Copy |
|---|---|---|
| 1 | Paused | `kith is paused until 15:10.` [Resume] |
| 2 | `problems` non-empty | the first problem's sentence, in `--danger` ink, plus its fix as a link |
| 3 | Exit offline | `Your exit, vps, is offline. Traffic is blocked.` / `…Traffic now goes directly.` |
| 4 | Join requests pending (admin) | `pixel-8 is asking to join home.` (for several: `2 devices are asking to join home.`) |
| 5 | Expected device unreachable | `nas can't be reached. Seen 3 h ago.` [Why?] |
| 6 | Only this device | `Only this device so far.` [Add a device] |
| 7 | Default | `Everything is working. 11 of 12 devices online.` |

- Default sub-line in `--ink-2`: `9 direct · 2 via vps · ↓ 3.4 MB/s ↑ 220 kB/s`.

**2. Needs you.** Up to 3 `NeedsYouCard`s, then `View all (5)`, which opens the header popover. The whole section is absent when empty.

**3. Devices.** Heading `Devices`, then up to 8 DeviceRows: this device first, then the others by name. Then `All 12 devices →`.

**4. Recent.** The last 5 Activity entries of kind Changes or Joins, then `All activity →`.

**Wide layouts:**
- At ≥1600 px, Recent becomes a persistent right column of 360 px showing the full Activity feed.
- Mobile uses the same order.

### 6.3 Devices (`/n/:net/devices`)

- **Toolbar:**
  - Search input `Filter by name or address` (`/` focuses it).
  - Chips: `All 12 · Online 11 · Offline 1 · Via relay 2 · Phones 2 · Exits 1` (Segmented, multi-select off).
  - On the right, `List | Map`.
  - Admins also see `+ Add device` here on mobile.
- **List:** a sticky column header in `--t-micro` small caps, then DeviceRows. Stable sort by name. `Sort: Name ▾` offers `Traffic (frozen)`, which reads `Sorted at 14:02 · Re-sort`.
- **Map:** see §9.2.
- **Empty (only this device):** `Only this device is in home.` [Add a device] [Add a phone].

### 6.4 Device detail (`/n/:net/devices/:dev`)

Sections in order, 32 px apart:

1. **Header:**
   - Name at `--t-title`. Admins can edit it inline: click the name, or use the pencil, which is always visible on touch.
   - StatusDot plus `online` / `offline`.
   - Mono address, Copy, then `name.home.kith.internal` with Copy.
   - Kind and OS as text if known.
   - Tags.
2. **Verdict:** one sentence plus a cause (table in §12.4), with an action only when one exists.
3. **Path:** a mini line diagram `This Mac ─ vps ─ nas` with RTT on the edge, endpoint (mono), MTU, `path_reason` sentence, and `reported by nas · 4 s ago`.
4. **Traffic:** Series with the range control, plus window totals `↓ 412 MB ↑ 38 MB in the last hour`.
5. **Names pointing here:** list with Copy.
6. **History:** the last 10 entries for this device, then `All →` (opens Activity with `device=`).
7. **Actions:**
   - Admin: `Rename`, `Use as exit` / `Allow as exit…`; then after 24 px of space, `Remove from home…` in danger-quiet.
   - Member: one `Ask an admin…` button in the same spot.
   - This device: `Leave home…` replaces Remove.

Keyboard: ↑/↓ in the split view moves between devices while focus is in the list. Esc closes the detail.

### 6.5 Activity (`/n/:net/activity`)

- **Toolbar:** Segmented `All · Changes · Joins · Paths · Blocked · Logs`, a device combobox and a search field. Everything syncs to the URL.
- **Feed:** `role="log"` with `aria-live=off`, virtualised, newest first, day dividers `Today`, `Yesterday`, `Mon 31 Oct`.
- **Grouping:**
  - Blocked entries with the same (src, dst, port) within 10 min become one row with a count.
  - Path flaps become `laptop switched paths 6 times since 14:02`.
- **Logs:** `debug` and `trace` hidden unless `Logs` is selected. `warn` and `error` always show. Logs render in mono 12 px, one line, click to expand, with level tags `INF WRN ERR`; `ERR` is in `--danger`.
- **Gap divider:** `Some events from 14:02 to 14:09 aren't available.`
- **Cap:** 2,000 entries. The bottom row reads `Older events aren't kept in this view.`
- **Linking:** clicking an entry about a device opens that device with `?at=<ms>&range=` chosen so the moment is in view, and the cursor parked there.

### 6.6 Names (`/n/:net/names`)

- Table: `name.home` (mono) · `→ target` · `set by laptop` (seal) · Copy · ⋯.
- `Add name` (admin) opens an inline row at the top, not a modal:
  - Name field with the `.home` suffix shown.
  - Value field. The record type is inferred: IP gives A or AAAA, a hostname gives CNAME. `More` reveals TXT.
  - Button `Add name`.
- Empty: `No names yet. Give a device a name like nas.home so others can reach it without its address.` [Add name]

### 6.7 Exits (`/n/:net/exits`)

**This device, at the top:** the heading `Send internet traffic through`, then a radio list (all options visible, not a dropdown):

- ○ `No exit · this device's own connection`
- ● `vps · 34 ms`
- ○ `nas · 12 ms`
- A row for an exit this device has no access to: in `--ink-3`, reads `You don't have access to laptop.` Admins see `Allow` inline; members see `Ask an admin`.

**Exits on the network** (admin view): for each exit, who has access, who is using it now, and live rate. Admin actions: `Allow device…`, `Stop offering…`.

### 6.8 Settings (`/n/:net/settings`) and This device (`/this`)

**Settings:**
- Appearance: Segmented `System · Light · Dark`, stored in localStorage (wrapped in try/catch). Compact rows (fine pointer only). Keyboard shortcuts on/off.
- Notifications: a master switch.
- Privacy: `Share link reports with home` (on), with the copy `Others see how this device connects (direct or relayed, latency), not what it sends.`
- Network: admins list, invites list with Revoke. Danger zone: `Leave home…`, `Delete home…` (typed confirmation).

**This device:** name, keys (fingerprint plus Copy), role per network, versions, the daemon log (mono, virtualised), and a `Copy redacted log` button. Redaction shortens IPs to `81.2.x.x` and shortens keys.

### 6.9 Gate screens (`Gate.svelte`, no app frame, centred, max 520 px)

- **Daemon down:**
  - `kith isn't running on this device.` / `Start it, then this page continues by itself.`
  - CopyField `sudo kith up`.
  - `Checking every 2 seconds…` in `--ink-3`.
  - Cross-fades into the app when the daemon answers.
- **Not in the group:**
  - `kith is running, but this user can't talk to it.`
  - CopyField `sudo usermod -aG kith $USER` (Linux) or `sudo dseditgroup -o edit -a $USER -t user kith` (macOS).
  - `Then log out and back in.`
- **Version skew:** `kith-ui 0.2 can't talk to kith 0.1. Update kith on this device.`
- **401** (server-rendered, no JS required):
  - `This page needs a fresh link.` / `For safety, kith only opens for the person at this computer. Run this in a terminal:` / `kith ui` with Copy / `The link it opens works once.`
  - Variants: `This link was already used. Links work once.` and `This link expired. They last 2 minutes.`
  - After a kith-ui restart under an open tab: an overlay over the dimmed last view reading `kith-ui restarted. Run kith ui to continue.`

---

## 7. User flows (exact copy)

### 7.1 First run: start a network

1. On Welcome, the user picks **Start a network**. The CreateNetwork sheet opens.
2. **Field:**
   - Label `Network name`, prefilled and selected `home`.
   - Helper (live): `Devices will be reachable as studio-mac.home.kith.internal`.
   - Typing a capital lowercases it. A space becomes a hyphen and shows `Spaces become hyphens.` for 1 s.
   - Validation goes through `/api/validate/name`. Taken: `You already have a network called home.`
3. **Button:** `Create "home"`. Pending label `Creating…`.
4. **Success:** the sheet becomes:
   - `home is ready. You're its admin.`
   - `Add your next device:` [Laptop or server] [Phone or tablet] · `Later`.
   - "Later" lands on Overview, whose headline reads `Only this device so far.`

### 7.2 Join a network (this device)

1. Welcome → **Join a network**. The JoinNetwork sheet has one input, `Invite link`.
2. On paste, the link is parsed: `Join home, invited by studio-mac. Link valid for 47 more minutes.` Button `Join home`.
3. **Errors:**
   - `This invite expired 2 hours ago. Ask studio-mac for a new one.`
   - `This invite has already been used. Ask for a new one.`
   - `Can't reach any device in home yet. Trying… (12 s)`. After 60 s, add: `Still no answer. Is one of home's devices online?`
4. **No link:** `I don't have a link — ask to join` shows:
   - The command an admin can approve: `Your request: kith join home@studio-mac` (if known).
   - `CheckCode`, labelled `Check code: K7Q·4XM — the admin will see the same code.`
   - Live states: `Waiting for an admin of home…`, then `You're in. 6 devices online.` (the page enters the network), or `An admin denied this request.`

### 7.3 Add a laptop or server (admin)

1. Entry: `+ Add device`, ⌘K "Add device", or the empty state. The AddDevice sheet asks `What are you adding?` [Laptop or server] [Phone or tablet].
2. Laptop or server shows Tabs: **Approve it live** (default) · **Send an invite link**.
3. **Approve it live:**
   - `Not installed yet?` CopyField with the install command.
   - `On the new device, run:` CopyField `kith join home@studio-mac`.
   - `Waiting for a request…` with a static ring glyph, no spinner loop. The sheet listens for `JoinRequested`.
4. **When a request arrives**, the sheet swaps in a NeedsYouCard (join):
   ```
   dev-box wants to join home
   Says: Linux                              (--ink-3 "Says:")
   Seen: from 203.0.113.7 · key k2c1·77f0
   Check code   K7Q·4XM
   Approve only if the new device shows this code.
   Name [dev-box     ]
                              [Deny]   [Approve]
   ```
   - Deny sits on the left, Approve (primary) on the right, 12 px apart.
   - If two requests share a name, an amber band reads: `Two devices asked to join as dev-box. Approve only the one whose code matches.`
5. **Approve:**
   - Button pending → `Signed. Spreading · 2 of 3 devices` (with acks) or `Saved on this device` (without).
   - Then `dev-box is online · direct · 14 ms` and `Show device`.
   - The sheet closes 2 s after the device is online, unless the pointer or focus is inside it.
   - Approved devices join as members only. Promotion is a separate tier-3 action.
6. **Deny:** instant, then `Denied dev-box · Undo` for 8 s.
7. **Lost the race:** `Approved by laptop · just now`. The buttons are removed, not disabled. A 409 maps to the same text.
8. **Expired:** `This request expired. Ask dev-box to try again.`

### 7.4 Invite link

1. Fields:
   - `Valid for` Segmented `1 hour · 1 day · 1 week` (default 1 hour).
   - `Uses` Segmented `1 · 5 · Unlimited` (default 1).
   - Button `Create invite` (tier 1: no extra confirmation).
2. **Result:**
   - CopyField with the link (mono).
   - QrPanel. The QR is not secret-gated for invites, but `Show QR code` is still a click so it never appears during screen sharing.
   - `Or run: kith join <link>`.
   - Live status: `Not used yet · expires 15:42`, then `Used by dev-box · online · direct`.
   - `Revoke invite` (instant, Undo).

### 7.5 Allow a blocked connection

1. **Card:** `phone → nas :22 (ssh) · blocked 4× in the last 10 min` with [Ignore] [Allow this…].
2. **Allow this…** opens a popover anchored to the card (desktop) or a sheet (mobile):
   - `Add rule: phone may reach nas on TCP 22.`
   - `Nothing else changes.` (from preview `new_paths`; if more than one: `This opens 3 new paths: …`)
   - `Need something broader? Edit policy` (link, v1.1).
   - Buttons [Cancel] [Add rule].
3. **After signing:** `Allowed by you · spreading 3 of 5`, then `phone can reach nas:22 now.` with Undo for 10 s. Undo signs the inverse op: `Undo signs a change that removes this rule.`
4. **Ignore:** `Hidden for a day. Still blocked.`
5. **Member view:** the same card with `Ask an admin` (§7.10).

### 7.6 Add a phone

The AddPhone sheet is one scrolling page with three numbered labels, no wizard.

1. **Name it.**
   - `Name` prefilled `phone` (or `phone-2` if taken).
   - Helper: `Will be phone.home.kith.internal`.
   - Tags field (default `phone`).
   - Button `Create phone config`. This is the only ledger write in the flow.
2. **Install WireGuard.** `Install the free WireGuard app (App Store · Google Play), tap +, then Scan from QR code.`
3. **Scan.**
   - Segmented `Just my devices | All traffic`. Helper: `Reach home's devices.` / `Everything, through vps.`
   - If no exit is allowed, `All traffic` is disabled with `No exit yet. Set one up in Exits →`.
   - Button `Show QR code (contains this phone's private key)`. Revealing it is **tier 3**: OS auth first, in the server.
   - Under `pointer:coarse` or below 640 px, `Download .conf` is primary and the QR is secondary. `Show config as text` is a disclosure.
4. **Status line under the QR:**
   - `Waiting for phone to connect… Turn on the tunnel in WireGuard after scanning.`
   - After 90 s: `Not connected yet. Check the tunnel is on, and that studio-mac is online. Phones connect through it.`
   - First handshake: `phone is connected · through studio-mac · 38 ms`. The QR collapses.
   - All traffic with no exit traffic 20 s after the handshake: `No internet on the phone? Turn the tunnel off and on once.`
5. **Abandoned flow:** the phone stays as a waiting row with `Show QR code again` and `Remove`. After 24 h a health card reads `phone never connected. Remove it?`

### 7.7 Something doesn't work

1. The user opens the device (row, map node, or ⌘K by name, address or `name:port`).
2. The verdict sentence (§12.4) is the answer.
   - Relayed: `Learn why` expands the `path_reason` sentence inline.
   - No `Try again` button until force-handshake exists.

### 7.8 Switch exit

1. Pick an option in ExitPicker (header chip, Exits screen or ⌘K "Use exit…").
2. The change is **optimistic**: the radio moves now and the row shows `Switching…`.
3. **Done:** `Your internet now goes through vps.` The header shows `Exit · vps`.
4. **Fail:** the radio reverts and inline text reads `Couldn't switch: vps isn't reachable right now. You're still using nas.`
5. **Back:** pick `No exit`. No undo toast.

### 7.9 Remove a device (admin, tier 2)

1. Detail → `Remove from home…` opens an AlertDialog:
   ```
   Remove phone from home?
   phone loses access to every device within seconds.
   Its address 100.64.0.7 and the name phone.home are freed.
   To come back it must join again.
   [ ] phone was lost or stolen — also revoke its invites and names
   (if applicable) laptop and tv use phone as their exit. Their internet will go directly.
                                       [Cancel]  [Remove phone]
   ```
   - Focus starts on Cancel. Enter is not bound to Remove. No typing is required.
2. **Progress in the row:** `Removing… 3 of 5 have dropped phone`, then `2 offline devices will drop it when they're back.` (shown as fact, not as a warning).
3. **Restrictions:**
   - The last admin can't be removed: `desk is the only admin. Make another device admin first.`
   - Removing **this** device or leaving the network is tier 3: OS auth plus typing the network name.
4. **Removed mid-dialog by someone else:** the dialog closes and a toast reads `phone was removed by desk while you were confirming. Nothing was signed.`

### 7.10 Member: Ask an admin (v1)

The `Ask an admin…` sheet:

- `Only admins can change this. Admins in home: desk (online), nas (offline).`
- `Send them this:` CopyField with the exact request, e.g. `kith allow phone nas 22` or the name to set.
- No fake "sent" state.
- v2 replaces this with a signed member request once the daemon kind exists.

### 7.11 Rename

1. Click the name. It becomes an inline field with the same validation and live DNS preview. Enter saves, Esc reverts.
2. **Taken:** `laptop is taken in home. Use laptop-2?` [Use laptop-2].
3. **DNS records point at the device:** `2 names point to dev-box. They'll follow the new name.`
4. **Concurrent edit band:** `desk changed this a second ago: "nas" → "storage".` [Use theirs] [Keep mine].

### 7.12 Notifications (browser)

- Off by default.
- After an event that would have notified arrives while the tab is hidden, an inline card appears at the top of Activity:
  - `Get told when something needs you?`
  - `kith can notify you when a device asks to join or your exit goes offline, even when this tab is in the background.`
  - [Turn on notifications] [Not now]. "Not now" hides it for 7 days.
- **Notifies:**
  - Join request (admins only): `"pixel-8" wants to join home` / `Open kith to check the code and approve.` Its only action is to open the page at `?need=`.
  - Exit offline.
  - A recently-used device went offline.
  - Daemon stopped for more than 10 s.
- **Never notifies:** path changes, records, logs, online events.
- **Rules:**
  - `tag` per kind and subject.
  - More than 3 in 30 s coalesce into one.
  - Never while the tab is focused.
  - Never for entries older than 30 s.

---

## 8. Realtime behaviour

### 8.1 Boot

1. Parse `#boot` and render complete data on the first frame (target under 50 ms). No skeletons.
2. Open `EventSource('/api/events')`, passing `Last-Event-ID` as `boot:seq` (via a query fallback on first connect).

### 8.2 Store (`store.svelte.ts`)

A normalised `$state`:

- `networks`, `peers[net][node]`, `links`, `records`, `pending`, `journal` (ring of 2000), `traffic` (columnar `Float64Array` per peer).
- `conn: 'live'|'catching_up'|'stale'|'daemon_down'|'ui_gone'|'unauth'`.
- `boot`, `seq`, `serverOffset`.

Every reducer is keyed on `EntryKind`, with a TS `never` exhaustiveness check. Entries with `seq <= snapshot.seq` are ignored. Ordering is by `seq`, never by `at_ms`.

### 8.3 Commit

- State entries apply to the store in the next rAF (bursts batch).
- Numbers (rates, RTT, sparklines) commit only on `tick`, all together.
- Interim states between frames are never painted.

### 8.4 Highlights

- A state change (online/offline, path kind, exit, record add/remove) gives the element the 4 s `--changed` tint.
- Numbers never tint, except when RTT more than doubles and stays that way for 3 ticks.
- Hidden-tab changes are collected and tinted on return, and the title shows `(n) `.

### 8.5 Connection states

| State | Trigger | UI |
|---|---|---|
| live | tick within 5 s | nothing |
| catching_up | reopened, replaying | nothing for 1 s, then `Reconnecting…` in the header (`--ink-3`) |
| stale | no tick for 5 s | after 3 more s: values stay, rates show `—`, dots go hollow, `as of 14:02` appears, actions get `aria-disabled` with inline text `Can't change anything until kith is back.` Nothing is queued |
| daemon_down | `event: daemon_down` | after 3 s: banner (`--attn-wash`) `kith isn't running on this device. Showing what was true at 14:02.` with a `How to start it` disclosure |
| ui_gone | EventSource error and `/api/status` fails | banner `This page lost its connection to kith-ui. Run kith ui to reopen it.` |
| unauth | 401 | overlay `kith-ui restarted. Run kith ui to continue.` |
| reset | `event: reset` | full refetch, toast `kith restarted at 14:03. Everything is up to date.` |

- Reconnect backoff: 0.5 s, 1 s, 2 s, then 4 s with jitter. Retry immediately on `visibilitychange`.
- On recovery, the banner collapses over 200 ms and everything changed in between gets the tint.

### 8.6 Actions

**Optimistic:** exit choice and UI state.

**Ledger ops:** pending, then accepted, then seen.
- Pending: spinner after 150 ms; the row reads `Signing…`; other controls on the row are disabled.
- Accepted: the 2xx.
- Seen: the journal entry plus acks, which drive the `Spreading · n of m` text.
- After 10 s with no seen: `Saved on this device. Waiting for other devices to hear about it.`
- A 409 from a race shows the other admin's result, not an error.

**`timing.ts` constants:**

| Constant | Value |
|---|---|
| tick | 2000 ms |
| stale | 5000 ms |
| banner delay | 3000 ms |
| tint | 4000 ms |
| row animation | 160 ms |
| removed hold | 1500 ms |
| spinner delay / minimum | 150 / 300 ms |
| seen timeout | 10 000 ms |
| undo | 8000 ms (10 000 ms for allow-rule) |
| path hysteresis | 5000 ms |
| sort freeze after pointer leaves | 1000 ms |
| notification coalesce | 3 in 30 s |
| replay notification cutoff | 30 s |
| announcer gap | 3000 ms |
| journal cap | 2000 |
| QR auto-hide | 300 000 ms |

### 8.7 Stability

- Lists never reorder while hovered or focused.
- A row inserted above the viewport keeps scroll anchored.
- The journal pauses once scrolled more than 48 px from the top. A pill reads `12 new ↑`; clicking it or pressing Home resumes.

---

## 9. Data visualisation

### 9.1 Traffic

- **Rate** = Δbytes/Δt between samples. Negative deltas are dropped. Gaps over 5 s break the line.
- **Resampling:** max per bucket, W/2 buckets. The detail chart adds a faint min–max band.
- **Units:** bytes SI for rates and totals. `idle` below 1 kB/s.
- **Live x-axis:** the series group translates left by one sample over the 2 s period. Under reduced motion it steps instead.
- **Y-scale:** grows immediately when a new peak passes the top. Shrinks after 10 s of more than 3× headroom, over 300 ms.
- **Correlation is mandatory:** event pins, relayed bands, and the two-way link between journal and chart.
- **Selection:** when a series is selected in a multi-series context, it turns `--accent` and the others drop to `--ink-4`.

### 9.2 Map (`layout.ts`, a pure function of the node-id set)

**Layout:**
- This device sits at the centre with a 14 px node and the label `this Mac` / `this device`.
- Peers sit on an outer ring. Angle = rank of `hash(NodeId)`, evenly spaced. When a device is added, the others slide to their new angles over 400 ms; nobody jumps across the ring.
- Relays sit on an inner ring at the mean angle of the peers that use them, drawn as 8 px squares.
- Phones hang off their gateway as satellites 28 px outward.
- Offline devices keep their slot and are drawn as a hollow ring in `--ink-3` with the label `laptop · 2 h ago`.
- More than 24 peers: online devices on the inner ring, offline on the outer. More than 60: the Grid (table) view by default, with the message `60 devices — showing as a table.`
- No localStorage positions. No simulation.

**Edges:**
- Only your own links by default.
- Focusing a peer (hover, focus or tap) draws its reported links at full strength and dims the rest to 25%.
- `Show all links` toggle, only below 15 devices.
- Direct: solid 1.5 px `--ink-2`. Relayed: dashed `4 3`, routed through the relay. Gateway: dotted `1 2`.
- Width 1.5–3.5 px on a sqrt scale of rate. Members get bands instead.
- Colour: amber when RTT is over 150 ms or loss over 2%; red when the device should be reachable and isn't (handshake stale for more than 3 min while reported online). Otherwise neutral.
- RTT label shows only on the focused device's links.

**Nodes:**
- 10 px circles: filled `--ink-1` for online, ring for offline.
- Labels outside the ring, 12 px, truncated at 14 characters.
- No device-type icons.

**Focus card** (popover):
```
laptop                       online · direct
100.64.0.7 · laptop.home.kith.internal
↓ 3.2 MB/s ↑ 140 kB/s        [spark]
23 ms · 192.168.1.20:51820
LAN address answered first
reported by laptop · 4 s ago       [Open device]
```

**Interaction:**
- Click opens the same detail component as the list.
- Arrow keys move around the ring.
- Zoom and pan with drag, plus `+`, `−` and `Fit` buttons.

**Accessibility:** each node is `<g role="button" tabindex>` with an aria-label like "laptop, online, direct, 23 milliseconds". `MapTable` is a visually hidden twin (from) · (to) · path · latency; the Grid view is that table made visible.

---

## 10. Keyboard and the command palette

- **Global shortcuts** (single-key ones can be turned off in Settings):
  - `⌘K` / `Ctrl K` palette.
  - `/` focuses the filter.
  - `?` opens the shortcut sheet.
  - `g o`, `g d`, `g a` go to Overview, Devices, Activity.
  - `Esc` closes the topmost layer.
  - `A` / `D` approve or deny, only while a join card has focus.
- **Palette** (Bits Command):
  - Groups: **Devices** (name, address, `name:port`) · **Actions** (`Add device`, `Add a phone`, `New invite`, `Use exit…`, `Add name`, `Rename nas`, `Copy nas address`) · **Go to** · **Networks**.
  - Prefix `>` limits to actions, `@` to devices.
  - Members see admin actions as `Ask an admin to rename nas…`.
  - Ranking: recency, then fuzzy match.
- Every pointer action has a keyboard path. Focus returns to the element that opened a layer.

---

## 11. Accessibility (merge gates)

1. **Announcer:**
   - One polite live region, one message at a time, at least 3 s apart, with repeats coalesced.
   - Announce only: join requests, blocked groups (at most once per triple per 5 min), results of the user's own actions, and this device's connectivity.
   - The assertive region is used only for daemon lost and recovered.
2. **Focus:** `:focus-visible` ring everywhere. `scroll-padding-top: 72px; scroll-padding-bottom: 80px`. A `Skip to content` link. Live data never moves focus or opens a dialog.
3. **Contrast:** a token test checks every pair: text ≥4.5:1, non-text ≥3:1. Both themes.
4. **Targets:** 44×44 on touch, at least 24 px on fine pointers. No gesture-only or hover-only actions. Every row has a visible ⋯.
5. **Colour is never the only carrier:** every state has a word and a shape.
6. **CI:** axe-core at 360, 768, 1280 and 1920 in both themes, with zero violations.
7. **Keyboard-only Playwright flows**, each under 20 keys: approve a join, allow a blocked connection, add a phone, set an exit.
8. **Manual checks before each release:** VoiceOver with Safari, Orca with Firefox, VoiceOver on iOS.
9. **Layout and performance:** CLS = 0 after first paint (PerformanceObserver in CI). WebKitGTK 2.40 on Ubuntu 22.04 runs in CI from M1.

---

## 12. Voice and copy (`copy.ts`)

### 12.1 Rules

- Sentence case. Verb-plus-object buttons.
- No `!`, `successfully`, `please`, `oops`, `Error:`, `Loading…`. A unit test fails any string containing them.
- Name the actor. Numbers carry units.
- Errors state cause, then the next step.

### 12.2 Vocabulary (canon; rename the CLI where it differs)

| Use | Never |
|---|---|
| device | peer, node |
| network | mesh |
| this device / `you` | — |
| admin | — |
| member | — |
| path: direct / via vps / stealth via vps | — |
| relay | — |
| exit | exit node |
| policy | — |
| blocked connection | dropped |
| request | — |
| invite | — |
| name (record only in the advanced view) | — |
| activity | journal |
| logs (raw daemon output) | — |
| history (the ledger) | — |
| online / offline | — |
| latency | — |
| check code | — |

### 12.3 Activity templates

| Entry | Template |
|---|---|
| PeerOnline | `phone came online · direct · 18 ms` |
| PeerOffline | `tv went offline` |
| PathChanged | `phone now relayed through vps (was direct)` |
| EndpointChanged | `phone changed network` |
| DeviceJoined | `pixel-8 joined · approved by laptop` (seal) |
| DeviceRemoved | `tv removed by laptop` (seal) |
| ExitChanged | `This device now uses vps as exit` |
| Dropped (grouped) | `Blocked phone → nas:22 · 4 times` |
| Warning | glyph plus the given text |
| Log | `INF` / `WRN` / `ERR` tag plus the line |

### 12.4 Verdicts (device detail)

| Situation | Copy |
|---|---|
| Direct | `Reachable directly · 12 ms` |
| Relayed | `Reachable through vps · 84 ms.` plus the path_reason sentence |
| Offline, seen under 10 min ago | `Not reachable right now. Seen 4 min ago. It may be asleep or switching networks.` |
| Offline, seen over 1 day ago | `Not seen for 3 days.` plus `Remove from home…` |
| Small MTU | `Reachable, but large transfers may stall (path MTU 1280).` |
| Blocked by policy | `Reachable, but policy blocks this device → nas on port 22.` plus `Allow this…` |
| Online but quiet (handshake older than 3 min) | `Online · quiet` |

### 12.5 Error mapping

| Code | Copy |
|---|---|
| PermissionDenied | `Only admins can do this. This device is a member.` |
| NotFound | `tv is no longer in this network.` |
| 409 | `laptop changed this a moment ago. Showing the latest.` |
| InvalidArgument | field-specific, e.g. `Names can use a–z, 0–9 and hyphens.` |
| Internal | `kith couldn't do this: <message>. Details are in Logs.` |

---

## 13. Friction tiers

| Tier | Actions | Friction |
|---|---|---|
| 0 | rename, add or remove a name, revoke an unused invite, deny a join, choose an exit | instant; Undo where reversible |
| 1 | approve a join, allow a blocked connection, create an invite, create a phone config | an in-place preview or confirm showing what will be signed |
| 2 | remove a device, allow an exit for a device | AlertDialog with consequences; the button names the object |
| 3 | add or remove an admin, remove this device or leave, delete a network, reveal a phone's private config | tier 2 plus typed name (except the reveal) plus server-side OS auth; the page shows `Waiting for confirmation on this computer…` |

---

## 14. What not to build

**Out entirely:**
- Astro, Electron, egui, gpui, Iced, Slint.
- d3-force or any physics layout.
- A WebSocket transport.
- An Inbox route. Needs-you lives on Overview, the header popover and the sheets.
- An icon-only desktop rail.

**Out of the visual language:**
- Green "online" dots. Colouring relayed paths.
- Pulsing or looping animations, crawling edge dashes, counting numbers.
- Skeletons on localhost.
- Empty "All clear" boxes, illustrations, OS or brand logos.
- Icons beside worded buttons.
- `backdrop-filter`.
- Theme toggle in the header. Density toggle beyond compact under a fine pointer.

**Out of the behaviour:**
- Toasts for network events.
- Approve buttons inside notifications.
- Press-and-hold QR reveal.
- Disabled submit buttons used for validation.
- Disabled admin controls for members.
- Optimistic ledger writes. Queued actions while stale.
- City or geo-IP lookups. OS shown as fact (it is shown only as a claim, "Says:").
- Buttons whose endpoint doesn't exist: Try again (force handshake), Pause, member request sending.
- Two separate traffic graphs for different windows. Stacked-area overview charts. Gauges, donuts.

**Deferred to v1.1 or v2:**
- Access screen: policy as sentences plus the reachability matrix; a 3-way diff only on 409.
- Global host and IP search.
- Pause with a deadline.
- Per-kind notification settings.
- Generic member requests.
- The Tauri shell is M2, not deferred.

---

## 15. Milestones

- **M1 (web):** Welcome, Overview, Devices (list, map, detail), Activity, Names, Exits, Settings, This device, Gates; flows 7.1–7.12.
- **M1.1:** Access (read sentences, then edit with the matrix).
- **M2 (Tauri):**
  - Native tray menu: `● Connected to home · 11 of 12 devices`, exit radio items, `1 needs you…`, `Open kith`, `Quit`.
  - Icon states: solid (fine), dot (needs you), hollow (off).
  - Native notifications with `Review` only.
  - Autostart, offered once: `Keep kith in the menu bar when you log in?`
  - `kith://join/<code>` deep links.
  - The webview is destroyed when the window closes.

---

## 16. Workstreams (7, independent)

Shared interfaces are frozen in week 1, in this order:

1. `types.gen.ts` (A).
2. The store's public API: `live.*` readonly plus `actions.*` (C).
3. `tokens.css`, `copy.ts` keys and the `ui/` component props (B).

Each workstream lands behind the fixture server, so nobody waits for the daemon.

**A. Server and contract.** Owns `crates/management/kith-ui-core/**`, `crates/management/kith-ui/src`, the `ts-rs` export in `kith-ipc`, and the `docs/plans/kith-ui-api.md` updates.
- Order:
  1. session and guards;
  2. snapshot with `boot`/`seq`, plus boot JSON inlining;
  3. SSE (boot:seq, tick, reset, gap, daemon_down);
  4. read APIs;
  5. write APIs with 409 and `based_on`;
  6. validate;
  7. osauth;
  8. `--fixture` replay.
- Exposes: the HTTP and SSE contract in §2, and `fixtures/office.json`.
- Also coordinates the daemon asks (check code, acks, from/to paths, link loss) with the join-request and map branches.

**B. Design system.** Owns `web/src/lib/tokens.css`, `tokens.ts`, `tailwind.css`, `lib/ui/**`, `lib/icons/**`, fonts, the ESLint palette ban, and the contrast test.
- Order: tokens → Button, Input, Field, Segmented, CopyField → Dialog, AlertDialog, Sheet, Drawer, Popover, Tooltip, Menu → Toast → Command → icons and the custom glyphs.
- Exposes: the component props documented in each component's header comment, plus a Storybook-free `/__kit` route in dev only.

**C. Live data layer.** Owns `lib/live/**`, `lib/api.ts`, `lib/format.ts`, `lib/announce.ts`, `timing.ts`.
- Order: boot parse → store → SSE client with reconnect and states → reducers (exhaustive) → rAF/tick commit → action runner (pending, accepted, seen, 409) → change-tint tracker → notification rules.
- Exposes: `live` (readonly runes), `actions.<verb>(args) → ActionHandle{state, error, seen}`, `format.*`, `announce()`.

**D. Shell and core screens.** Owns `App.svelte`, `router.ts`, `routes/Overview|Devices|Device|Activity|Settings|ThisDevice|Gate|Welcome`, `lib/kith/{DeviceRow,PathBadge,Rate,Rtt,Headline,IdentityBlock,RoleChip,ConnectionBanner,EventRow,NeedsYouCard}`, the palette wiring, and `copy.ts` (owns the file; others add keys by PR to D).
- Depends on B and C interfaces only.

**E. Flows and sheets.** Owns `sheets/**`, `routes/Names.svelte`, `routes/Exits.svelte`, `lib/kith/{QrPanel,CheckCode,ExitPicker,AskAdminSheet}`.
- Order: AddDevice (approve live) → Invite → AllowBlocked → RemoveDevice → AddPhone → CreateNetwork / JoinNetwork → Names → Exits.
- Uses only `actions.*` and `live.*`.

**F. Visualisation.** Owns `lib/charts/**` and `lib/map/**`.
- Order: `resample.ts` (unit-tested: gaps, resets, max-bucket) → Spark painter → Series with pins and bands → `map/layout.ts` (a pure function, property-tested for stability) → Map.svelte → MapTable.
- Exposes:
  - `<Spark peer>`
  - `<Series peer range events>`
  - `<Map net focused onSelect>`

**G. Quality, build and shell.** Owns `web/tests/**`, CI workflows, `size-limit`, `build.rs`, `fallback.html`, release packaging, and later `apps/kith-app/**` (M2).
- Order: CI build plus embed → Playwright on the fixture → axe at 4 widths × 2 themes → 4 keyboard flows → CLS check → WebKitGTK runner → M2 Tauri tray, notifications, autostart, deep link.

**Collision rules:**
- Only B edits `lib/ui` and tokens.
- Only C edits `lib/live`.
- Only D merges `copy.ts`.
- Only A edits Rust.
- Screens never call `fetch` directly; they go through C.
- No component hard-codes a colour, a string or a duration.
