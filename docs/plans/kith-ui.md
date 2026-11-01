# kith-ui: plan

A live control room for a kith network: what is happening right now, what happened, and what needs a decision. Not a form for each CLI command.

## The main decision: no manager node

The first draft had one always-on device serve the UI to everyone. That copies the shape of Tailscale's admin console, and it fits kith badly:

- It puts a central point back into a network designed not to have one.
- If two devices run it, which one is "the" UI? Their pages could disagree.
- It acts with the manager's key on behalf of whoever is looking. Handing a page to someone hands them the manager's power, and the history records the manager, not the person.

kith already has the right primitive: a ledger that every device holds, where every change is signed by the device that makes it and checked by every other device. So:

**Every device runs its own UI, for its own user, and acts only with its own key.**

- It listens on `127.0.0.1` only, behind a one-time token, like Jupyter: `kith ui` opens the browser already signed in. Nothing on the network can reach it.
- An admin's UI can change the network; a member's UI is read-only, and asks an admin when it needs something (see requests below). This isn't enforced by the UI but by the ledger: every other device rejects an operation its author had no right to sign, whatever program produced it.
- Two, ten or every device can run it. They all show the same network, because they read the same replicated ledger and the same shared live data. Two admins editing at once is the case the ledger already handles: last writer wins per name or policy, removal wins over a concurrent action, and the UI shows "changed by laptop a second ago" when the thing you're editing moves under you.
- "Handing control" to someone means making their device an admin, a signed, visible step in the history, not sharing a page.

What's left from the manager idea is useful as an option: an always-on device can publish a **read-only** status page on its mesh address for devices that can't run the UI (a phone's browser, a TV). It shows the shared view, takes no actions, and is off by default.

## What it shows, live

Most of this doesn't exist in the daemon yet; the CLI gets it too (below).

**Now**
- A **map of the network**: devices as nodes, real links between them coloured by kind (direct, relayed, through which relay), with latency on each link. Built from each device's own report of who it reaches and how, shared over the control channel.
- **Traffic per device and per link**, as live graphs (last minute, hour), not byte totals.
- **Who is connected right now**, joins and leaves as they happen, path changes ("phone moved from wifi to 4G, now relayed through vps").
- **Exits in use**: which devices send their internet traffic through which exit, and how much.

**Needs you**
- **Join requests.** A new device scans the network's request code and appears on every admin's screen within a second: its name, its OS, where it's coming from. Approve or deny; no invite link to send around. (Invites stay for when no admin is watching.)
- **Blocked connections**: "phone tried to reach nas on port 22 and the policy blocked it", with one click to allow that and nothing more. This turns the policy from a file you write into something you shape from what actually happens.
- **Health**: `kith doctor` findings, continuously: a device that hasn't been seen for a day, a phone with a too-small path, a relay that's down, an invite that's about to be used up.

**History**
- **Timeline of the network** from the ledger: who added whom, policy changes, exits granted, names set, each with the device that signed it. Nobody else can show this honestly, because nobody else has a signed history.
- **Event log** for this device: handshakes, path changes, drops, warnings, searchable and filterable, kept for a while on disk.
- **Connection log**: which device talked to which, on what port, how much, when. Kept on the device that saw it.

**Doing**
- Add a device (request code or invite, with QR), add a phone (both QR codes), remove, rename, names, exits, the policy editor with a preview of who can reach what, all with the result shown live as it spreads to other devices.
- Desktop notifications for the things that need you.

## Privacy and trust in shared data

The live view needs devices to share more than they do now (links, latency, traffic totals). Rules:

- What a device reports about itself is signed by it and shown as "reported by laptop", never mixed up with ledger facts.
- Connection logs and per-connection detail never leave the device that saw them by default. A network can opt in, per policy, to sharing them with admins.
- A member that isn't an admin sees the shared view only to the extent the policy allows.
- Admin actions from the UI on an admin device can ask for the OS's own confirmation (Touch ID, the system password), so a laptop left unlocked isn't a network left open.

## Work in the daemon first

The UI is a view; the data has to exist first, and the CLI should get the same things.

1. **Event journal.** Every notable event (peer online and offline, path and endpoint changes, joins, removals, ledger operations with their author, policy drops with source, destination and port, exit and relay use, warnings) gets a sequence number and goes into a bounded ring with a disk spill. Clients subscribe with filters and resume from a sequence number, so nothing is missed across a reconnect. This replaces today's thin event stream.
2. **Metrics sampler.** Per peer and per link: bytes, packets, latency, sampled every second, kept for an hour in memory, coarser for a day.
3. **Connection table** from the data plane: open flows with their counters, and closed ones into the journal.
4. **Log capture**: the daemon's own log lines into the same journal, so logs are part of the live view, not a file on disk somewhere.
5. **Presence of links**: each device gossips a small signed summary of its links (who, how, latency) with the rest, the way it gossips endpoints now.
6. **Join requests**: a request code (network id plus admins' addresses, no secret), a pending-request message over the control channel, and approval as an ordinary signed ledger operation.
7. **Policy drop reports** with enough detail to propose the matching rule.

CLI at the same time: `kith watch` on top of the journal (with `--since` and filters), `kith log`, `kith top` (live traffic per peer), `kith requests` / `kith approve`, `kith history`.

## The program

- **`kith-ui`**, a separate binary in this repo; `kith ui` starts it, the way `git foo` runs `git-foo`. The main binary stays small for routers, and the UI ships as its own download.
- Talks to the local daemon over the existing IPC socket, with the new subscription calls. Pushes to the browser over a WebSocket.
- The page is a real app: live graphs, the network map, instant updates. Built with a small framework (Svelte or Preact) at release time, and compiled into the binary, so installing it is still one file.
- Later, the same page inside a small desktop shell gives kith a menu bar icon and native notifications on macOS, and is the natural first step towards the Settings-app work.

## Steps

1. Event journal, metrics sampler and the subscription IPC, with the CLI's `watch`, `log` and `top` on top. Useful on its own, and the foundation for everything else.
2. `kith-ui` with the dashboard, timeline and live log, local only.
3. Link reports and the network map.
4. Join requests and approvals, in the CLI and the UI.
5. Policy drop reports and allow-from-the-UI; the policy editor.
6. Connection log; opt-in sharing with admins.
7. Optional read-only status page on the mesh; desktop shell.

## Open questions

- How long to keep history on small devices (routers have little disk)?
- Should link reports be on by default? They reveal who talks to whom to every member.
- Should join requests ever be approvable by a non-admin member the admins trust, or only by admins?
