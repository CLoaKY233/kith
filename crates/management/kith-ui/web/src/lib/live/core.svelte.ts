/* eslint-disable svelte/prefer-svelte-reactivity -- the plain Maps and Sets in this file
   are bookkeeping that no markup reads; everything markup reads is $state or SvelteMap. */

// The store's state and its live wiring (spec §8.1 to §8.5). store.svelte.ts makes the
// page's one instance and exposes it as `live`; tests make their own with fake deps.
//
// - Boot: the inline snapshot hydrates synchronously, so the first frame is complete.
// - Commit: entries and ticks queue as they arrive and apply together in the next frame;
//   numbers (rates, latency, sparklines) change only on a tick, all at once.
// - Order: by seq. An entry at or below the state's seq changes nothing; an entry at or
//   below the journal's seq is a duplicate and is dropped.
// - Resync: a gap, an unknown device or the daemon coming back refetches the snapshot;
//   entries arriving meanwhile wait and apply on top of it.
// - Connection: `conn` is the state of §8.5; `connShown` is what the page shows, after
//   that table's delays.

import type {
  Boot,
  Entry,
  ExitView,
  Gap,
  LinkTick,
  Pending,
  PeerInfo,
  RecordView,
  StatusView,
  Tick,
  TrafficSeries,
  Versions,
} from '../types.gen';
import type {
  ApiFailure,
  ConnState,
  GateReason,
  OpenInvite,
  PeerLive,
  PolicyView,
  Relay,
  ServerMode,
  TrafficColumns,
} from '../types';
import { deviceKey, tintKey } from './keys';
import { noEffects, omit, reduce, type Effects, type ReducerState } from './reducers';
import { Paths, type ShownPath } from './stability.svelte';
import type { StreamControl, StreamHandlers } from './stream';
import { Tints } from './tint.svelte';
import { Columns, merged } from './traffic';
import {
  BANNER_DELAY_MS,
  CATCHING_UP_QUIET_MS,
  DAEMON_POLL_MS,
  JOURNAL_CAP,
  RECONNECT_BACKOFF_MS,
  RTT_TINT_FACTOR,
  RTT_TINT_TICKS,
  STALE_MS,
} from './timing';

/** What the store needs from the outside; the page passes the browser's, tests fakes. */
export interface Deps {
  /** GET a JSON path; rejects with an ApiFailure. */
  get<T>(path: string): Promise<T>;
  /** Opens the event stream (stream.ts `connect`). */
  connect(resume: () => string, on: StreamHandlers): StreamControl;
  /** Runs `flush` before the next paint (requestAnimationFrame). */
  frame(flush: () => void): void;
  /** Whether the tab is visible now. */
  visible(): boolean;
  /** Calls back when the tab is shown or hidden. */
  watchVisibility(changed: (visible: boolean) => void): void;
}

/** Entries the stream could not deliver (§6.5 gap divider). */
export type JournalGap = {
  /** The first and last missing seq. */
  readonly from: number;
  readonly to: number;
  /** Server time of the last entry before the gap. */
  readonly fromMs: number;
  /** Server time of the first entry after it; `null` until one arrives. */
  readonly toMs: number | null;
  /** The divider sits before the entry at this position (see `journalTotal`). */
  readonly position: number;
};

const noPending = (): Pending => ({ joins: [], blocked: [] });

/** Gates the store keeps polling /api/boot under, every DAEMON_POLL_MS (§6.9). */
const POLLED_GATES: ReadonlySet<GateReason> = new Set(['daemon_down', 'not_in_group', 'version_skew']);

const SIGNED_OUT: ReadonlySet<string> = new Set(['unauthorized', 'session_used', 'session_expired']);

function failure(cause: unknown): Pick<ApiFailure, 'code' | 'status'> {
  if (typeof cause === 'object' && cause !== null && 'code' in cause && 'status' in cause) {
    const { code, status } = cause as ApiFailure;
    return { code, status };
  }
  return { code: 'internal', status: 0 };
}

function gateFor(code: ApiFailure['code']): GateReason | null {
  switch (code) {
    case 'daemon_down':
    case 'not_in_group':
    case 'version_skew':
    case 'unauthorized':
    case 'session_used':
    case 'session_expired':
      return code;
    default:
      return null;
  }
}

/** §2.12 `op_seen {op, seen_by, of}`, which the generated Entry type does not list yet. */
function opSeen(entry: Entry): { op: string; seen_by: number; of: number } | null {
  const e = entry as unknown as Record<string, unknown>;
  if (e['kind'] !== 'op_seen') return null;
  const { op, seen_by, of } = e;
  if (typeof op !== 'string' || typeof seen_by !== 'number' || typeof of !== 'number') return null;
  return { op, seen_by, of };
}

/** Runs `task` once at a time; a call while it runs makes it run once more after. */
function coalesced(task: () => Promise<void>): () => void {
  let running = false;
  let again = false;
  const run = (): void => {
    if (running) {
      again = true;
      return;
    }
    running = true;
    void task().finally(() => {
      running = false;
      if (again) {
        again = false;
        run();
      }
    });
  };
  return run;
}

export class Store implements ReducerState {
  // §8.2: identity of this boot.
  boot = $state('');
  /** The last seq applied to the state below. */
  seq = $state(0);
  /** Server clock minus this clock, in ms, taken from the boot snapshot. */
  serverOffset = $state(0);
  /** §8.5 connection state, as it is. */
  conn = $state<ConnState>('catching_up');
  /** The connection state the page shows: `conn` after §8.5's delays. */
  connShown = $state<ConnState>('live');
  /** Set when the page must show a Gate screen instead of the app. */
  gate = $state<GateReason | null>(null);
  /** False until the first snapshot arrives. */
  ready = $state(false);

  status = $state<StatusView | null>(null);
  versions = $state<Versions | null>(null);
  /** peers[net][node]. This device is never in it (see status.memberships). */
  peers = $state<Record<string, Record<string, PeerInfo>>>({});
  /** When each device (`net/node`) was last heard from, server ms; `null` if never. */
  seenAt = $state<Record<string, number | null>>({});
  /** links[net]: every reported link, from the latest tick. */
  links = $state.raw<Record<string, LinkTick[]>>({});
  /** records[net], loaded on demand by actions.loadRecords. */
  records = $state<Record<string, RecordView[]>>({});
  /** exits[net], loaded on demand by actions.loadExits. */
  exits = $state<Record<string, ExitView[]>>({});
  /** invites[net], loaded on demand by actions.loadInvites. */
  invites = $state<Record<string, OpenInvite[]>>({});
  /** policies[net], loaded on demand by actions.loadPolicy. */
  policies = $state<Record<string, PolicyView>>({});
  /** relays[net], loaded on demand by actions.loadRelays. */
  relays = $state<Record<string, Relay[]>>({});
  /** What this device offers to its networks, loaded on demand by actions.loadServer. */
  server = $state<ServerMode | null>(null);
  pending = $state<Pending>(noPending());
  /** Oldest first, at most JOURNAL_CAP entries, in seq order within a boot. */
  journal = $state.raw<Entry[]>([]);
  /** Entries ever appended; the newest entry's position is `journalTotal - 1`. */
  journalTotal = $state(0);
  gaps = $state.raw<JournalGap[]>([]);
  /** Numbers per `net/node`, committed together on each tick. */
  numbers = $state.raw<Record<string, PeerLive>>({});
  /** Columnar counters per `net/node`. Read `trafficVersion` to react to new samples. */
  traffic = new Map<string, Columns>();
  trafficVersion = $state(0);
  /** The server's hour of samples (GET /api/traffic) is in `traffic`, not only the boot
   * snapshot's last 5 minutes; until then charts show the older part as loading. */
  trafficLoaded = $state(false);
  /** Server time of the last tick, in ms. */
  lastTickMs = $state(0);
  /** Server time the data shown was last known true: `as of 14:02` (§8.5). */
  asOfMs = $state(0);
  /** When the daemon last restarted under this page, for the reset toast. */
  restartedAtMs = $state<number | null>(null);

  readonly tints: Tints;
  readonly paths: Paths;

  readonly #deps: Deps;
  #stream: StreamControl | null = null;
  #started = false;
  /** Seq of the newest entry received from the stream, for the resume point. */
  #receivedSeq = 0;
  /** Seq of the newest entry in the journal, for dropping duplicates. */
  #journalSeq = 0;
  /** The first snapshot's seq: entries up to it are history, replayed into the journal
   * only (§8.2: they change no state, notify no one and mark no gap). */
  #historySeq = 0;
  /** Entries and gap reports in arrival order, applied in the next frame. */
  #queue: ({ entry: Entry } | { gap: Gap })[] = [];
  #ticks: Tick[] = [];
  #scheduled = false;
  #syncing = false;
  #buffer: Entry[] = [];
  #listeners = new Set<(entry: Entry) => void>();
  #ackListeners = new Set<(op: string, by: number, of: number) => void>();
  #rtt = new Map<string, { base: number; over: number }>();
  #staleTimer: ReturnType<typeof setTimeout> | null = null;
  #staleRetries = 0;
  #shownTimer: ReturnType<typeof setTimeout> | null = null;
  #pollTimer: ReturnType<typeof setTimeout> | null = null;
  #probing = false;

  constructor(deps: Deps) {
    this.#deps = deps;
    this.tints = new Tints(deps.visible());
    this.paths = new Paths(
      () => this.now(),
      (key) => this.tints.mark(key),
    );
  }

  /** Server time now, in ms. */
  now(): number {
    return Date.now() + this.serverOffset;
  }

  get changed(): ReadonlyMap<string, number> {
    return this.tints.changed;
  }

  get unseen(): number {
    return this.tints.unseen;
  }

  /** Calls `listener` with each new journal entry, after the frame that applied it. */
  onEntry(listener: (entry: Entry) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  /** Calls `listener` with each propagation ack (§2.12 `op_seen`): `by` of `of` devices
   * have `op`. Acks are not journal entries the page shows. */
  onAck(listener: (op: string, by: number, of: number) => void): () => void {
    this.#ackListeners.add(listener);
    return () => this.#ackListeners.delete(listener);
  }

  /** Hydrates from `inline` (or GET /api/boot without one) and opens the stream. */
  /**
   * Starts from the inline snapshot, or fetches one. `signedOut` is why trading the link's
   * token failed (used, expired): without a session the boot read can only say
   * `unauthorized`, and the gate should say which.
   */
  async start(inline: Boot | null, signedOut: GateReason | null = null): Promise<void> {
    if (this.#started) return;
    this.#started = true;
    this.#deps.watchVisibility((visible) => this.#visibility(visible));
    if (inline) {
      this.#adopt(inline);
      this.#afterBoot();
      return;
    }
    await this.#fetchBoot();
    if (signedOut && this.gate === 'unauthorized') this.gate = signedOut;
  }

  /** Closes the stream and every timer. */
  stop(): void {
    this.#stream?.close();
    this.#stream = null;
    for (const timer of [this.#staleTimer, this.#shownTimer, this.#pollTimer]) {
      if (timer !== null) clearTimeout(timer);
    }
    this.#staleTimer = this.#shownTimer = this.#pollTimer = null;
    this.tints.clear();
    this.paths.clear();
  }

  /** Applies a snapshot as the first one, with no stream. For tests and the kit page. */
  hydrate(snapshot: Boot): void {
    this.#adopt(snapshot);
  }

  /** Refetches the snapshot, as after a change the stream does not report (a new network). */
  resync(): void {
    this.#resync();
  }

  /** Refetches /api/pending, as after an action that settles a request. */
  readonly refreshPending = coalesced(async () => {
    try {
      this.#setPending(await this.#deps.get<Pending>('/api/pending'), true);
    } catch {
      // The next entry or snapshot brings it.
    }
  });

  /** Loads the server's kept samples (§9.1: an hour per device) in front of the boot
   * snapshot's tail, so the 15m and 1h ranges are full from the first view. */
  readonly loadTraffic = coalesced(async () => {
    let history: TrafficSeries[];
    try {
      history = await this.#deps.get<TrafficSeries[]>('/api/traffic');
    } catch {
      return;
    }
    for (const series of history) {
      const key = deviceKey(series.network, series.node);
      if (!this.#knows(key)) continue;
      this.traffic.set(key, merged(series.samples, this.traffic.get(key) ?? new Columns()));
    }
    this.trafficLoaded = true;
    this.trafficVersion += 1;
  });

  /** Refetches /api/status: exit state, warnings and problems only the server knows. */
  readonly refreshStatus = coalesced(async () => {
    try {
      this.#setStatus(await this.#deps.get<StatusView>('/api/status'));
    } catch {
      // The next entry or snapshot brings it.
    }
  });

  // Boot and gates (§8.1, §6.9).

  async #fetchBoot(): Promise<void> {
    let snapshot: Boot;
    try {
      snapshot = await this.#deps.get<Boot>('/api/boot');
    } catch (cause) {
      const { code, status } = failure(cause);
      const gate = gateFor(code);
      if (gate) this.gate = gate;
      if (SIGNED_OUT.has(code)) this.#setConn('unauth');
      else if (status === 0) this.#setConn('ui_gone');
      this.#poll();
      return;
    }
    this.#adoptFresh(snapshot);
  }

  /** A snapshot from a gate poll: the first one, or one after the daemon came back. */
  #adoptFresh(snapshot: Boot): void {
    const fresh = !this.ready || snapshot.boot !== this.boot;
    this.#adopt(snapshot, this.ready ? (fresh ? 'reset' : 'resync') : 'boot');
    this.#afterBoot();
  }

  #afterBoot(): void {
    if (this.gate && POLLED_GATES.has(this.gate)) {
      this.#poll();
      return;
    }
    if (!this.trafficLoaded) this.loadTraffic();
    this.#setConn('live');
    this.#armStale();
    this.#openStream();
  }

  /** Polls /api/boot while gated or unreachable, until it answers (§6.9). */
  #poll(): void {
    const gated = this.gate !== null && POLLED_GATES.has(this.gate);
    if (!gated && this.conn !== 'ui_gone') return;
    if (this.#pollTimer !== null) clearTimeout(this.#pollTimer);
    this.#pollTimer = setTimeout(() => {
      this.#pollTimer = null;
      void this.#fetchBoot();
    }, DAEMON_POLL_MS);
  }

  /**
   * Replaces the state with a snapshot. `boot` is the first; `resync` is the same boot
   * after missed entries; `reset` is a new boot. After the first, everything that changed
   * is tinted (§8.5: on recovery, what changed in between gets the tint).
   */
  #adopt(snapshot: Boot, mode: 'boot' | 'resync' | 'reset' = 'boot'): void {
    const first = mode === 'boot';
    const before = first ? null : this.#outline();

    this.boot = snapshot.boot;
    this.seq = snapshot.seq;
    this.serverOffset = snapshot.server_time_ms - Date.now();
    if (first) {
      // The first connect asks for the server's ring, so Activity and Recent have the
      // history from the start; the state stays the snapshot's.
      const floor = Math.max(0, snapshot.seq - JOURNAL_CAP);
      this.#receivedSeq = floor;
      this.#journalSeq = floor;
      this.#historySeq = snapshot.seq;
    } else if (mode !== 'resync') {
      this.#receivedSeq = snapshot.seq;
      this.#journalSeq = snapshot.seq;
      this.#historySeq = 0;
    }
    this.#setStatus(snapshot.status);
    this.versions = snapshot.versions;
    this.gate = snapshot.versions.compatible ? null : 'version_skew';

    const peers: Record<string, Record<string, PeerInfo>> = {};
    const seenAt: Record<string, number | null> = {};
    for (const [net, list] of Object.entries(snapshot.peers)) {
      peers[net] = Object.fromEntries(list.map((peer) => [peer.node, peer]));
      for (const peer of list) {
        seenAt[deviceKey(net, peer.node)] =
          peer.last_seen_secs === null ? null : snapshot.server_time_ms - peer.last_seen_secs * 1000;
      }
    }
    this.peers = peers;
    this.seenAt = seenAt;
    this.#setPending(snapshot.pending, !first);

    if (first) this.traffic = new Map();
    for (const series of snapshot.traffic_tail) {
      const key = deviceKey(series.network, series.node);
      let columns = this.traffic.get(key);
      if (!columns) {
        columns = new Columns();
        this.traffic.set(key, columns);
      }
      for (const sample of series.samples) columns.record(sample.at_ms, sample.rx_bytes, sample.tx_bytes);
    }
    for (const key of [...this.traffic.keys()]) {
      if (!this.#knows(key)) this.traffic.delete(key);
    }
    if (snapshot.last_tick) {
      this.#commitTick(snapshot.last_tick);
    } else {
      this.#commitNumbers({ at_ms: snapshot.server_time_ms, traffic: [], links: [] });
      this.asOfMs = snapshot.server_time_ms;
    }

    for (const [net, list] of Object.entries(this.peers)) {
      for (const peer of Object.values(list)) {
        const key = deviceKey(net, peer.node);
        if (first) this.paths.seed(key, peer.path, snapshot.server_time_ms);
        else this.paths.observe(key, peer.path, snapshot.server_time_ms, []);
      }
    }
    if (before) this.#tintDifferences(before);
    this.ready = true;
  }

  /** What tints compare across a refetch. */
  #outline(): { peers: Map<string, boolean>; exit: string | null } {
    const peers = new Map<string, boolean>();
    for (const [net, list] of Object.entries(this.peers)) {
      for (const peer of Object.values(list)) peers.set(deviceKey(net, peer.node), peer.online);
    }
    return { peers, exit: this.status?.exit_node ?? null };
  }

  #tintDifferences(before: { peers: Map<string, boolean>; exit: string | null }): void {
    const now = new Set<string>();
    for (const [net, list] of Object.entries(this.peers)) {
      for (const peer of Object.values(list)) {
        const key = deviceKey(net, peer.node);
        now.add(key);
        if (before.peers.get(key) !== peer.online) this.tints.mark(tintKey.device(net, peer.node));
      }
    }
    for (const key of before.peers.keys()) {
      if (!now.has(key)) this.#forgetDevice(key);
    }
    if ((this.status?.exit_node ?? null) !== before.exit) this.tints.mark(tintKey.exit);
  }

  /** `net/node` back into its parts. Network ids never hold a `/`. */
  #split(key: string): [string, string] {
    const cut = key.indexOf('/');
    return [key.slice(0, cut), key.slice(cut + 1)];
  }

  #knows(key: string): boolean {
    const [net, node] = this.#split(key);
    return this.peers[net]?.[node] !== undefined;
  }

  #setStatus(status: StatusView): void {
    this.status = status;
    this.versions = status.versions;
    if (!status.versions.compatible) this.gate = 'version_skew';
    else if (this.gate === 'version_skew') this.gate = null;
  }

  #setPending(next: Pending, tintNew: boolean): void {
    if (tintNew) {
      const known = new Set([...this.pending.joins, ...this.pending.blocked].map((item) => item.id));
      for (const item of [...next.joins, ...next.blocked]) {
        if (!known.has(item.id)) this.tints.mark(tintKey.need(item.id));
      }
    }
    this.pending = next;
  }

  // The stream (§2.6, §8.5).

  #openStream(): void {
    if (this.#stream) return;
    this.#stream = this.#deps.connect(() => `${this.boot}:${this.#receivedSeq}`, {
      entry: (entry) => this.#receive(entry),
      tick: (tick) => this.#receiveTick(tick),
      reset: (reset) => void this.#reset(reset.started_at_ms),
      gap: (gap) => {
        this.#queue.push({ gap });
        this.#schedule();
      },
      daemonDown: () => this.#setConn('daemon_down'),
      daemonUp: () => {
        this.#setConn('catching_up');
        this.#armStale();
        this.#resync();
      },
      open: () => {
        if (this.conn === 'ui_gone') this.#setConn('catching_up');
        // Ticks must follow within STALE_MS of opening, as of any other moment.
        this.#waitForTick();
      },
      error: () => {
        if (this.conn === 'live') this.#setConn('catching_up');
        void this.#probe();
      },
    });
  }

  #receive(entry: Entry): void {
    if (entry.seq > this.#receivedSeq) this.#receivedSeq = entry.seq;
    const ack = opSeen(entry);
    if (ack) {
      for (const listener of this.#ackListeners) listener(ack.op, ack.seen_by, ack.of);
      return;
    }
    this.#queue.push({ entry });
    this.#schedule();
  }

  #receiveTick(tick: Tick): void {
    // kith-ui keeps ticking while the daemon is away; only `daemon_up` ends that state.
    if (this.conn !== 'daemon_down') this.#setConn('live');
    this.#armStale();
    this.#ticks.push(tick);
    this.#schedule();
  }

  /** `event: reset`: the daemon restarted. Refetch everything on a fresh stream. */
  async #reset(startedAtMs: number): Promise<void> {
    this.#stream?.close();
    this.#stream = null;
    this.#queue = [];
    this.#setConn('catching_up');
    let snapshot: Boot;
    try {
      snapshot = await this.#deps.get<Boot>('/api/boot');
    } catch {
      this.#pollTimer = setTimeout(() => void this.#reset(startedAtMs), DAEMON_POLL_MS);
      return;
    }
    this.#adopt(snapshot, 'reset');
    this.restartedAtMs = startedAtMs;
    this.#afterBoot();
  }

  /** Refetches the snapshot; entries that arrive meanwhile wait and apply on top. */
  readonly #resync = coalesced(async () => {
    this.#syncing = true;
    let snapshot: Boot | null = null;
    try {
      snapshot = await this.#deps.get<Boot>('/api/boot');
    } catch {
      snapshot = null;
    }
    this.#syncing = false;
    const waiting = this.#buffer;
    this.#buffer = [];
    if (snapshot && snapshot.boot !== this.boot) {
      // The daemon restarted while the stream could not say so; the snapshot's time is
      // the closest known to when.
      void this.#reset(snapshot.server_time_ms);
      return;
    }
    if (snapshot) this.#adopt(snapshot, 'resync');
    const fx = noEffects();
    for (const entry of waiting) this.#apply(entry, fx);
    this.#effects(fx);
  });

  /** After a stream error: is kith-ui there, and does it still know this page? */
  async #probe(): Promise<void> {
    if (this.#probing) return;
    this.#probing = true;
    try {
      this.#setStatus(await this.#deps.get<StatusView>('/api/status'));
      if (this.conn === 'ui_gone') this.#setConn('catching_up');
    } catch (cause) {
      const { code, status } = failure(cause);
      if (SIGNED_OUT.has(code)) {
        // A new link is needed; retrying cannot help (§6.9).
        this.#setConn('unauth');
        this.#stream?.close();
        this.#stream = null;
      } else if (code === 'daemon_down') {
        this.#setConn('daemon_down');
      } else if (status === 0) {
        this.#setConn('ui_gone');
      }
    } finally {
      this.#probing = false;
    }
  }

  /** Waits STALE_MS for a tick; without one, reconnects, and keeps reconnecting on the
   * §8.5 backoff while the stream stays silent (a reopened stream can be dead too). */
  #armStale(): void {
    this.#staleRetries = 0;
    this.#waitForTick();
  }

  #waitForTick(): void {
    if (this.#staleTimer !== null) clearTimeout(this.#staleTimer);
    const steps = RECONNECT_BACKOFF_MS;
    const backoff = this.#staleRetries === 0 ? 0 : (steps[Math.min(this.#staleRetries, steps.length) - 1] ?? 0);
    this.#staleTimer = setTimeout(() => {
      this.#staleTimer = null;
      if (this.conn === 'live' || this.conn === 'catching_up') this.#setConn('stale');
      else if (this.conn !== 'stale') return;
      // The tick is the heartbeat: without one the connection may be dead but open.
      this.#staleRetries += 1;
      this.#stream?.reconnect();
      this.#waitForTick();
    }, STALE_MS + backoff);
  }

  #setConn(next: ConnState): void {
    if (this.conn === next) return;
    // Once the page needs a new link nothing else matters (§8.5 unauth).
    if (this.conn === 'unauth') return;
    this.conn = next;
    if (this.#shownTimer !== null) clearTimeout(this.#shownTimer);
    this.#shownTimer = null;
    const delay =
      next === 'catching_up'
        ? CATCHING_UP_QUIET_MS
        : next === 'stale' || next === 'daemon_down'
          ? BANNER_DELAY_MS
          : 0;
    if (delay === 0) {
      this.connShown = next;
      return;
    }
    this.#shownTimer = setTimeout(() => {
      this.#shownTimer = null;
      this.connShown = this.conn;
    }, delay);
  }

  #visibility(visible: boolean): void {
    this.tints.setVisible(visible);
    // A frame requested while visible does not run in a hidden tab; flush anyway, so
    // entries keep applying (and part 2's notifications see them) in the background.
    if (!visible && this.#scheduled) setTimeout(this.#flush, 0);
  }

  // Commit (§8.3).

  #schedule(): void {
    if (this.#scheduled) return;
    this.#scheduled = true;
    if (this.#deps.visible()) this.#deps.frame(this.#flush);
    else setTimeout(this.#flush, 0);
  }

  readonly #flush = (): void => {
    if (!this.#scheduled) return;
    this.#scheduled = false;
    const queue = this.#queue;
    const ticks = this.#ticks;
    this.#queue = [];
    this.#ticks = [];

    const added: Entry[] = [];
    let gaps = this.gaps;
    const fx = noEffects();
    for (const item of queue) {
      if ('gap' in item) {
        // History older than the ring holds is not a gap in what this page saw.
        if (item.gap.missed_to <= this.#historySeq) continue;
        const last = added.at(-1) ?? this.journal.at(-1);
        gaps = [
          ...gaps,
          {
            from: item.gap.missed_from,
            to: item.gap.missed_to,
            fromMs: last?.at_ms ?? this.asOfMs,
            toMs: null,
            position: this.journalTotal + added.length,
          },
        ];
        // State missed those entries: refetch it. Entries after this one wait for it.
        this.#resync();
        continue;
      }
      const { entry } = item;
      if (entry.seq <= this.#journalSeq) continue;
      this.#journalSeq = entry.seq;
      const open = gaps.at(-1);
      if (open && open.toMs === null) gaps = [...gaps.slice(0, -1), { ...open, toMs: entry.at_ms }];
      added.push(entry);
      if (this.#syncing) this.#buffer.push(entry);
      else this.#apply(entry, fx);
    }
    if (gaps !== this.gaps) this.gaps = gaps;
    if (added.length > 0) this.#append(added);
    ticks.forEach((tick, i) => this.#commitTick(tick, i === ticks.length - 1));
    this.#effects(fx);
    for (const entry of added) {
      if (entry.seq <= this.#historySeq) continue;
      for (const listener of this.#listeners) {
        try {
          listener(entry);
        } catch (error) {
          // A listener's fault must not stop the store; report it where errors go.
          setTimeout(() => {
            throw error;
          }, 0);
        }
      }
    }
  };

  #apply(entry: Entry, fx: Effects): void {
    if (entry.seq <= this.seq) return;
    this.seq = entry.seq;
    reduce(this, entry, fx);
  }

  #append(added: readonly Entry[]): void {
    const next = [...this.journal, ...added];
    this.journal = next.length > JOURNAL_CAP ? next.slice(next.length - JOURNAL_CAP) : next;
    this.journalTotal += added.length;
    const oldest = this.journalTotal - this.journal.length;
    if (this.gaps.some((gap) => gap.position < oldest)) {
      this.gaps = this.gaps.filter((gap) => gap.position >= oldest);
    }
  }

  #effects(fx: Effects): void {
    for (const key of fx.removed) this.#forgetDevice(key);
    for (const key of fx.tint) this.tints.mark(key);
    for (const [key, change] of fx.paths) this.paths.observe(key, change.path, change.atMs, change.flaps);
    if (fx.resync) this.#resync();
    if (fx.pending) this.refreshPending();
    if (fx.status) this.refreshStatus();
  }

  #forgetDevice(key: string): void {
    this.traffic.delete(key);
    this.#rtt.delete(key);
    this.paths.forget(key);
    this.tints.forget(tintKey.device(...this.#split(key)));
    this.tints.forget(tintKey.rttOf(key));
    if (key in this.numbers) this.numbers = omit(this.numbers, key);
    if (key in this.seenAt) this.seenAt = omit(this.seenAt, key);
  }

  /** Records a tick's samples; the last tick of a frame also commits its numbers. */
  #commitTick(tick: Tick, numbers = true): void {
    for (const sample of tick.traffic) {
      const key = deviceKey(sample.network, sample.node);
      if (!this.#knows(key)) continue;
      let columns = this.traffic.get(key);
      if (!columns) {
        columns = new Columns();
        this.traffic.set(key, columns);
      }
      if (columns.record(tick.at_ms, sample.rx, sample.tx) && columns.heard()) {
        this.seenAt[key] = tick.at_ms;
      }
    }
    this.lastTickMs = tick.at_ms;
    this.asOfMs = tick.at_ms;
    if (numbers) this.#commitNumbers(tick);
    this.trafficVersion += 1;
  }

  /** Rates, latency and link facts for every device, all at once (§8.3). */
  #commitNumbers(tick: Tick): void {
    const self = this.status?.node ?? null;
    const byNet: Record<string, LinkTick[]> = {};
    const own = new Map<string, LinkTick>();
    for (const link of tick.links) {
      (byNet[link.network] ??= []).push(link);
      if (link.from === self) own.set(deviceKey(link.network, link.to), link);
    }
    const numbers: Record<string, PeerLive> = {};
    for (const [net, list] of Object.entries(this.peers)) {
      for (const node of Object.keys(list)) {
        const key = deviceKey(net, node);
        const link = own.get(key);
        const rates = this.traffic.get(key)?.rates() ?? null;
        const previous = this.numbers[key];
        numbers[key] = {
          rx: rates?.rx ?? previous?.rx ?? 0,
          tx: rates?.tx ?? previous?.tx ?? 0,
          rttMs: link?.rtt_ms ?? null,
          loss: link?.loss ?? null,
          band: link?.rate_band ?? null,
          path: link?.path ?? null,
          relay: link?.relay ?? null,
        };
        this.#watchRtt(net, node, link?.rtt_ms ?? null);
      }
    }
    this.links = byNet;
    this.numbers = numbers;
  }

  /** §8.4: a latency cell tints when RTT more than doubles and stays so for 3 ticks. */
  #watchRtt(net: string, node: string, rtt: number | null): void {
    if (rtt === null) return;
    const key = deviceKey(net, node);
    const seen = this.#rtt.get(key);
    if (!seen) {
      this.#rtt.set(key, { base: rtt, over: 0 });
      return;
    }
    if (rtt > seen.base * RTT_TINT_FACTOR) {
      seen.over += 1;
      if (seen.over >= RTT_TINT_TICKS) {
        this.tints.mark(tintKey.rtt(net, node), false);
        seen.base = rtt;
        seen.over = 0;
      }
    } else {
      seen.base = rtt;
      seen.over = 0;
    }
  }
}

/** Read-only view of the store. Assigning through it is a type error. */
export interface Live {
  readonly boot: string;
  readonly seq: number;
  readonly serverOffset: number;
  readonly conn: ConnState;
  readonly connShown: ConnState;
  readonly gate: GateReason | null;
  readonly ready: boolean;
  readonly status: StatusView | null;
  readonly versions: Versions | null;
  readonly peers: Readonly<Record<string, Readonly<Record<string, PeerInfo>>>>;
  readonly seenAt: Readonly<Record<string, number | null>>;
  readonly links: Readonly<Record<string, readonly LinkTick[]>>;
  readonly records: Readonly<Record<string, readonly RecordView[]>>;
  readonly exits: Readonly<Record<string, readonly ExitView[]>>;
  readonly invites: Readonly<Record<string, readonly OpenInvite[]>>;
  readonly policies: Readonly<Record<string, PolicyView>>;
  readonly relays: Readonly<Record<string, readonly Relay[]>>;
  readonly server: ServerMode | null;
  readonly pending: Pending;
  readonly journal: readonly Entry[];
  readonly journalTotal: number;
  readonly gaps: readonly JournalGap[];
  readonly numbers: Readonly<Record<string, PeerLive>>;
  readonly traffic: ReadonlyMap<string, TrafficColumns>;
  readonly trafficVersion: number;
  readonly trafficLoaded: boolean;
  readonly lastTickMs: number;
  readonly asOfMs: number;
  readonly restartedAtMs: number | null;
  /** Tinted elements, key → stamp (see keys.ts `tintKey`). */
  readonly changed: ReadonlyMap<string, number>;
  /** Paths as badges show them, per `net/node`, after hysteresis. */
  readonly paths: ReadonlyMap<string, ShownPath>;
  readonly unseen: number;
  /** Server time now, in ms. */
  now(): number;
}

/** The read-only `live` view of a store. */
export function viewOf(store: Store): Live {
  return {
    get boot() { return store.boot; },
    get seq() { return store.seq; },
    get serverOffset() { return store.serverOffset; },
    get conn() { return store.conn; },
    get connShown() { return store.connShown; },
    get gate() { return store.gate; },
    get ready() { return store.ready; },
    get status() { return store.status; },
    get versions() { return store.versions; },
    get peers() { return store.peers; },
    get seenAt() { return store.seenAt; },
    get links() { return store.links; },
    get records() { return store.records; },
    get exits() { return store.exits; },
    get invites() { return store.invites; },
    get policies() { return store.policies; },
    get relays() { return store.relays; },
    get server() { return store.server; },
    get pending() { return store.pending; },
    get journal() { return store.journal; },
    get journalTotal() { return store.journalTotal; },
    get gaps() { return store.gaps; },
    get numbers() { return store.numbers; },
    get traffic() { return store.traffic; },
    get trafficVersion() { return store.trafficVersion; },
    get trafficLoaded() { return store.trafficLoaded; },
    get lastTickMs() { return store.lastTickMs; },
    get asOfMs() { return store.asOfMs; },
    get restartedAtMs() { return store.restartedAtMs; },
    get changed() { return store.changed; },
    get paths() { return store.paths.shown; },
    get unseen() { return store.unseen; },
    now: () => store.now(),
  };
}
