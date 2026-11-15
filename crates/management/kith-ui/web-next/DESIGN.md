# kith dashboard — design spec (v2)

Supersedes BRIEF.md's design sections. Read this whole file before touching code.

## Principles
1. **Every element earns its place.** If it doesn't help someone answer a question or do a task,
   it goes. No decorative numbering, no internal jargon ("tick", "fixture"), no duplicate views.
2. **One answer per screen.** Each screen answers one question; the title says which.
3. **Calm by default, loud only when something needs attention.** Black and white; red only for
   something wrong or destructive; nothing else is coloured.
4. **Data is honest and readable.** Every chart has a unit, sensible scale, axis ticks and a
   hover readout. Never let one outlier (a counter reset) flatten a chart.
5. **Same thing, same look.** One component per pattern, from `src/ui`. Screens never hand-roll
   buttons, headers, toasts, badges, empty states or charts.
6. **Motion explains, never decorates.** 150–200 ms ease-out; page enter fade + 4px rise;
   no infinite animations except live packet flow on the map. Respect prefers-reduced-motion.

## Information architecture
Top bar (left → right): logo + network name · primary nav · live status · ⌘K.
Primary nav has five items:

| Item | Path | Question it answers |
|---|---|---|
| Overview | `/` | Is everything okay, and does anything need me? |
| Devices | `/devices` (List) and `/devices?view=map` (Map) | What's on my network and how is each reached? |
| Stats | `/stats` | How is the network performing over time? (Grafana-style) |
| Activity | `/activity` | What happened, and when? |
| Settings | `/settings/<section>` | Configuration: Network, Exits, Names, Access, People |

- Device page: `/devices/<name>`.
- Settings has a left sub-nav (vertical list) with the five sections; `/settings` → `/settings/network`.
- Narrow screens (< 900px): nav collapses into a menu button that opens a UI SDK Drawer.
- ⌘K palette reaches every page, every device and the common actions.

## Visual system (tokens in src/styles.css)
- Colours: ink-0 #0a0a0a page · ink-2 #171717 raised · line #262626 hairline · line-2 #3a3a3a
  strong hairline · fg #fafafa · fg-2 #a4a4a4 secondary · fg-3 #6f6f6f tertiary · hot #f2555a
  (errors/destructive only). Charts may use a small greyscale ramp + white; never hues.
- Type: Geist (UI) and Geist Mono (numbers, addresses, ids, labels).
  Page title 40px/450/-0.035em · section title 20px/500 · body 14px · secondary 13px ·
  label: mono 11px uppercase tracking .08em fg-3 · big number: mono 32–40px.
- Spacing: 4px grid. Page padding 32px (16px mobile). Sections 48px apart. Content max 1280px.
- Surfaces: hairline boxes, square corners, no shadows, no gradients, no blur.
- Status: 7px square — filled white = online/ok, outlined grey = offline, filled red = problem.

## Components (src/ui — the only place these live)
- `Page` — title, optional description (one sentence), optional actions (right), children.
- `Section` — title, optional description, optional actions; no numbering.
- `Panel` — hairline box with optional header (label + right slot) and body; `PanelGrid` lays
  panels out sharing hairlines.
- `Stat` — label, value (mono), optional unit and secondary line.
- `Button` — variants primary (white), secondary (outline), ghost, danger; sizes sm/md; icon slot;
  busy state.
- `Badge` — mono outline label; tone neutral | strong | danger.
- `StatusDot` — online | offline | problem.
- `Empty` — icon, one line, optional action.
- `Copy` — mono text that copies on click, with a brief "copied" state.
- `toast()` + `<Toaster/>` — one toast system.
- Charts (`src/ui/charts`):
  - `TimeSeries` — multi-series line/area, responsive (ResizeObserver), nice y ticks with units,
    x time ticks, gridlines, crosshair + tooltip listing every series at that time, legend that
    toggles series, robust scaling (clip the scale to the 99th percentile when one point is
    an outlier and mark clipped points), empty state.
  - `Spark` — tiny inline line for rows.
  - `BarList` — ranked horizontal bars with value labels.
  - `Meter` — straight segmented meter.
UI SDK components are used for every interactive control (Tabs, SegmentedControl, Select,
Switch, Input, Dialog, AlertDialog, Drawer, Command, Combobox, Toggle, Tooltip, HoverCard,
Timeline, Waterfall, Meter, NumberInput) with their own styles (web.css); add only layout classes.

## Screen specs
- **Overview**: one-line health sentence as the subtitle ("All 10 online devices are healthy" or
  "1 device needs attention"); "Needs you" list (join requests, blocked connections in plain
  English, network problems) — hidden when empty; a row of four Stats (Online, Throughput now,
  Exit, Paths direct/relayed); one TimeSeries "Traffic, last 15 minutes"; "Busiest devices"
  BarList (top 5) and "Recent changes" (5 events, link to Activity). No constellation.
- **Devices**: header actions = view switch (List/Map, SegmentedControl) + "Add device".
  List: search, filter (All/Online/Offline/Relayed/Phones), sort; table columns Name, Address,
  Path, Latency, Traffic (Spark), Last seen. Map: full-height canvas filling the content area,
  legend in a corner panel, filter + zoom controls in a toolbar, side panel on select.
- **Device**: header (name, status, path, actions Rename/Remove in a menu); Stats row (Latency,
  Received, Sent, Last seen); Path diagram; TimeSeries traffic (range 1h/6h/24h); Internet
  traffic settings (exit Select, Lock, Pause, Save); History (Timeline, this device).
- **Stats** (new, Grafana-style): range picker (15m · 1h · 6h · 24h, SegmentedControl) and
  auto-refresh indicator; a 12-column panel grid:
  1. Total throughput (rx/tx area) — wide.
  2. Throughput by device (top 6 lines + legend toggle) — wide.
  3. Latency by link (rtt ms lines, from link ticks buffered in the client for the session).
  4. Packet loss by link (%).
  5. Connection paths over time (direct/relayed/stealth counts).
  6. Devices online over time (from the journal).
  7. Busiest devices (BarList for the range).
  8. Drops by reason (BarList) if the status reports drops.
  Panels have a header (title + current value) and the chart; hover readouts sync across
  panels (shared crosshair time).
- **Activity**: filters (category Toggles + device Combobox); 24h density strip; sessions
  Waterfall; Timeline by day. Plain sentences, no raw log noise by default.
- **Settings/Network**: Invite a device (expiry, uses, Create → link + QR), Open invites,
  Relays, This device (exit/relay switches).
- **Settings/Exits**: exits list (name, status, users now, rate, Allowed switch); diagram only
  if it adds information.
- **Settings/Names**: records table + add; automatic device names.
- **Settings/Access**: rules as sentences, Matrix, Raw TOML (Tabs); Blocked connections.
- **Settings/People**: people list + Add person (Console mode); clear local-mode message.

## Copy
Sentence case. Short. No exclamation marks. Numbers with units. Times relative ("2m ago")
with absolute on hover (title). Never show raw TOML/rule syntax outside the Raw editor.

## Quality bar
`npx tsc -b --noEmit` clean; no unused exports/files/imports; no console errors; every screen
works at 1440 and 390 wide; keyboard reachable; no layout shift while data loads (skeleton or
reserved space).

## For agents rebuilding a screen (v2 pass)
- Read DESIGN.md (this file), src/ui/README.md and src/ui/index.ts. Build ONLY from src/ui
  components + UI SDK components. Never hand-roll a button, header, badge, toast, empty state,
  stat or chart. Use `fmt` from src/ui for every number/time.
- Routes changed (see src/App.tsx and src/nav.ts): Map is `/devices?view=map`; settings pages are
  `/settings/<network|exits|names|access|people>` and render inside the settings sub-nav (so
  those screens render only their own content: a `Page` without the outer settings chrome).
- Delete the old helper files your screen used once nothing imports them (check with grep);
  remove dead code; keep types strict. Never run git, kith, cargo, a VPN or a server
  (one dev server is already running for the lead; you can't use it). Check with
  `npx tsc -b --noEmit`.
- Each screen must handle: loading (Skeleton, no layout shift), empty, error, and 390px width.
- Report: files changed/deleted, what the screen shows (by DESIGN.md's spec), tsc result.
