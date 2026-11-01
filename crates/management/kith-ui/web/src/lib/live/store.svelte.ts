// The single store (spec §8.2). Public surface, frozen for the other workstreams:
//
//   live.*     readonly reactive state: read it in markup or $derived, never assign it.
//   actions.*  every change the page can make; each returns an ActionHandle.
//   start()    hydrates from the boot snapshot and opens the stream; main.ts calls it.
//   notifications  the browser-notification opt-in (§7.12) for Settings and Activity.
//
// Actions run through runner.ts; watch.svelte.ts follows the journal and connection to
// mark actions seen, notify and announce, so the stream code needs no hooks for them.
//
// Workstream C owns this file and its internals. This first version hydrates from the
// boot snapshot (inlined `#boot`, or GET /api/boot under the dev and fixture servers),
// applies stream entries through reducers.ts and commits tick numbers together.

import { SvelteSet } from 'svelte/reactivity';
import { go, href } from '../../router';
import { api, ApiError } from '../api';
import type {
  Boot,
  Entry,
  ExitView,
  Invite,
  LinkTick,
  Pending,
  PeerInfo,
  RecordView,
  StatusView,
  Tick,
  Versions,
} from '../types.gen';
import type { ConnState, GateReason, PeerLive, TrafficColumns } from '../types';
import { createActions, type Actions } from './actions';
import { Notifications } from './notifications.svelte';
import { browserEnv } from './notify';
import { reduce, type ReducerState } from './reducers';
import { Runner } from './runner';
import { connect } from './stream';
import { JOURNAL_CAP, STALE_MS, TINT_MS, TRAFFIC_SAMPLES } from './timing';
import { watch } from './watch.svelte';

const EMPTY_PENDING: Pending = { joins: [], blocked: [] };

class Columns implements TrafficColumns {
  at = new Float64Array(TRAFFIC_SAMPLES);
  rx = new Float64Array(TRAFFIC_SAMPLES);
  tx = new Float64Array(TRAFFIC_SAMPLES);
  length = 0;

  push(at: number, rx: number, tx: number): void {
    if (this.length === TRAFFIC_SAMPLES) {
      this.at.copyWithin(0, 1);
      this.rx.copyWithin(0, 1);
      this.tx.copyWithin(0, 1);
      this.length -= 1;
    }
    this.at[this.length] = at;
    this.rx[this.length] = rx;
    this.tx[this.length] = tx;
    this.length += 1;
  }
}

class Store implements ReducerState {
  // §8.2 identity of this boot.
  boot = $state('');
  seq = $state(0);
  /** Server clock minus this clock, in ms; add it to Date.now() for server time. */
  serverOffset = $state(0);
  conn = $state<ConnState>('catching_up');
  /** Set when the page must show a Gate screen instead of the app. */
  gate = $state<GateReason | null>(null);
  /** False until the first snapshot arrives. */
  ready = $state(false);

  status = $state<StatusView | null>(null);
  versions = $state<Versions | null>(null);
  /** peers[net][node]. */
  peers = $state<Record<string, Record<string, PeerInfo>>>({});
  /** links[net]: every reported link, from the latest tick. */
  links = $state.raw<Record<string, LinkTick[]>>({});
  /** records[net], loaded on demand by actions.loadRecords. */
  records = $state<Record<string, RecordView[]>>({});
  /** exits[net], loaded on demand by actions.loadExits. */
  exits = $state<Record<string, ExitView[]>>({});
  /** invites[net], loaded on demand by actions.loadInvites. */
  invites = $state<Record<string, Invite[]>>({});
  pending = $state<Pending>(EMPTY_PENDING);
  /** Newest last, at most JOURNAL_CAP entries, ordered by seq. */
  journal = $state.raw<Entry[]>([]);
  /** Seq ranges the stream reported missing (§6.5 gap divider). */
  gaps = $state.raw<{ from: number; to: number; at_ms: number }[]>([]);
  /** Numbers per `net/node`, committed together on each tick. */
  numbers = $state.raw<Record<string, PeerLive>>({});
  /** Columnar counters per `net/node`. Read `trafficVersion` to react to new samples. */
  traffic: Map<string, Columns> = new Map();
  trafficVersion = $state(0);
  /** Server time of the last tick, in ms. */
  lastTickMs = $state(0);
  /** Keys tinted right now (§8.4). */
  changed = new SvelteSet<string>();
  /** Changes collected while the tab was hidden, for the `(n)` title prefix. */
  unseen = $state(0);

  /** Applies a boot snapshot, replacing everything. */
  hydrate(snapshot: Boot): void {
    this.boot = snapshot.boot;
    this.seq = snapshot.seq;
    this.serverOffset = snapshot.server_time_ms - Date.now();
    this.status = snapshot.status;
    this.versions = snapshot.versions;
    const peers: Record<string, Record<string, PeerInfo>> = {};
    for (const [net, list] of Object.entries(snapshot.peers)) {
      peers[net] = Object.fromEntries(list.map((p) => [p.node, p]));
    }
    this.peers = peers;
    this.pending = snapshot.pending;
    this.traffic = new Map();
    for (const series of snapshot.traffic_tail) {
      const columns = new Columns();
      for (const s of series.samples) columns.push(s.at_ms, s.rx_bytes, s.tx_bytes);
      this.traffic.set(`${series.network}/${series.node}`, columns);
    }
    if (snapshot.last_tick) this.commitTick(snapshot.last_tick);
    this.trafficVersion += 1;
    this.gate = snapshot.versions.compatible ? null : 'version_skew';
    this.ready = true;
  }

  applyEntry(entry: Entry): void {
    if (entry.seq <= this.seq) return;
    this.seq = entry.seq;
    reduce(this, entry);
    const next = this.journal.length >= JOURNAL_CAP ? this.journal.slice(1) : this.journal.slice();
    next.push(entry);
    this.journal = next;
    if (entry.kind === 'join_requested' || entry.kind === 'blocked') void this.refreshPending();
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') this.unseen += 1;
    this.scheduleUntint();
  }

  commitTick(tick: Tick): void {
    this.lastTickMs = tick.at_ms;
    const numbers: Record<string, PeerLive> = {};
    const linksByNet: Record<string, LinkTick[]> = {};
    const self = this.status?.node ?? null;
    for (const link of tick.links) {
      (linksByNet[link.network] ??= []).push(link);
    }
    for (const t of tick.traffic) {
      const key = `${t.network}/${t.node}`;
      let columns = this.traffic.get(key);
      if (!columns) {
        columns = new Columns();
        this.traffic.set(key, columns);
      }
      const prevAt = columns.length > 0 ? (columns.at[columns.length - 1] ?? 0) : 0;
      const prevRx = columns.length > 0 ? (columns.rx[columns.length - 1] ?? 0) : 0;
      const prevTx = columns.length > 0 ? (columns.tx[columns.length - 1] ?? 0) : 0;
      columns.push(tick.at_ms, t.rx, t.tx);
      const dt = (tick.at_ms - prevAt) / 1000;
      const own = linksByNet[t.network]?.find((l) => l.to === t.node && (self === null || l.from === self));
      numbers[key] = {
        rx: prevAt > 0 && dt > 0 && t.rx >= prevRx ? (t.rx - prevRx) / dt : 0,
        tx: prevAt > 0 && dt > 0 && t.tx >= prevTx ? (t.tx - prevTx) / dt : 0,
        rttMs: own?.rtt_ms ?? null,
        loss: own?.loss ?? null,
        band: own?.rate_band ?? null,
        path: own?.path ?? null,
        relay: own?.relay ?? null,
      };
    }
    this.links = linksByNet;
    this.numbers = numbers;
    this.trafficVersion += 1;
  }

  async refreshPending(): Promise<void> {
    try {
      this.pending = await api.get<Pending>('/api/pending');
    } catch {
      // The next entry or tick retries; nothing to show.
    }
  }

  private untint: ReturnType<typeof setTimeout> | null = null;
  private scheduleUntint(): void {
    if (this.changed.size === 0 || this.untint !== null) return;
    this.untint = setTimeout(() => {
      this.untint = null;
      this.changed.clear();
    }, TINT_MS);
  }
}

const store = new Store();

/** Read-only view of the store. Assigning through it is a type error. */
export interface Live {
  readonly boot: string;
  readonly seq: number;
  readonly serverOffset: number;
  readonly conn: ConnState;
  readonly gate: GateReason | null;
  readonly ready: boolean;
  readonly status: StatusView | null;
  readonly versions: Versions | null;
  readonly peers: Readonly<Record<string, Readonly<Record<string, PeerInfo>>>>;
  readonly links: Readonly<Record<string, readonly LinkTick[]>>;
  readonly records: Readonly<Record<string, readonly RecordView[]>>;
  readonly exits: Readonly<Record<string, readonly ExitView[]>>;
  readonly invites: Readonly<Record<string, readonly Invite[]>>;
  readonly pending: Pending;
  readonly journal: readonly Entry[];
  readonly gaps: readonly { from: number; to: number; at_ms: number }[];
  readonly numbers: Readonly<Record<string, PeerLive>>;
  readonly traffic: ReadonlyMap<string, TrafficColumns>;
  readonly trafficVersion: number;
  readonly lastTickMs: number;
  readonly changed: ReadonlySet<string>;
  readonly unseen: number;
  /** Server time now, in ms. */
  now(): number;
}

export const live: Live = {
  get boot() { return store.boot; },
  get seq() { return store.seq; },
  get serverOffset() { return store.serverOffset; },
  get conn() { return store.conn; },
  get gate() { return store.gate; },
  get ready() { return store.ready; },
  get status() { return store.status; },
  get versions() { return store.versions; },
  get peers() { return store.peers; },
  get links() { return store.links; },
  get records() { return store.records; },
  get exits() { return store.exits; },
  get invites() { return store.invites; },
  get pending() { return store.pending; },
  get journal() { return store.journal; },
  get gaps() { return store.gaps; },
  get numbers() { return store.numbers; },
  get traffic() { return store.traffic; },
  get trafficVersion() { return store.trafficVersion; },
  get lastTickMs() { return store.lastTickMs; },
  get changed() { return store.changed; },
  get unseen() { return store.unseen; },
  now: () => Date.now() + store.serverOffset,
};

const runner = new Runner(store);

/** Every change and on-demand read. Each returns an ActionHandle (§16 C, runner.svelte.ts). */
export const actions: Actions = createActions(store, runner);
export type { Actions };

/** Feeds a propagation ack (§2.12 `op_seen`) to the action that signed `op`. */
export function ackOp(op: string, by: number, of: number): void {
  runner.ack(op, by, of);
}

/** Browser notification opt-in and delivery (§7.12): Settings and the Activity card. */
export const notifications = new Notifications(
  browserEnv(live.now, (need) => {
    const name = need ? (store.status?.networks.find((n) => n.id === need.network)?.name ?? null) : null;
    if (need && name) go(`${href({ name: 'overview', net: name })}?need=${encodeURIComponent(need.id)}`);
  }),
);

watch(live, runner, notifications);

let stopStream: (() => void) | null = null;
let staleTimer: ReturnType<typeof setTimeout> | null = null;

function armStale(): void {
  if (staleTimer !== null) clearTimeout(staleTimer);
  staleTimer = setTimeout(() => {
    if (store.conn === 'live' || store.conn === 'catching_up') store.conn = 'stale';
  }, STALE_MS);
}

function readInlineBoot(): Boot | null {
  const node = typeof document === 'undefined' ? null : document.getElementById('boot');
  if (!node?.textContent) return null;
  try {
    return JSON.parse(node.textContent) as Boot;
  } catch {
    return null;
  }
}

/** Hydrates from `#boot` (or /api/boot) and opens the stream. Safe to call once. */
export async function start(): Promise<void> {
  if (stopStream) return;
  let snapshot = readInlineBoot();
  if (!snapshot) {
    try {
      snapshot = await api.get<Boot>('/api/boot');
    } catch (cause) {
      const code = cause instanceof ApiError ? cause.code : 'internal';
      store.conn = code === 'unauthorized' ? 'unauth' : 'ui_gone';
      store.gate =
        code === 'daemon_down' || code === 'not_in_group' || code === 'version_skew'
          ? code
          : code === 'unauthorized' || code === 'session_used' || code === 'session_expired'
            ? code
            : null;
      return;
    }
  }
  store.hydrate(snapshot);
  stopStream = connect(() => `${store.boot}:${store.seq}`, {
    entry: (entry) => store.applyEntry(entry),
    tick: (tick) => {
      store.conn = 'live';
      store.commitTick(tick);
      armStale();
    },
    reset: () => {
      void api.get<Boot>('/api/boot').then((fresh) => store.hydrate(fresh), () => undefined);
    },
    gap: (gap) => {
      store.gaps = [...store.gaps, { from: gap.missed_from, to: gap.missed_to, at_ms: live.now() }];
    },
    daemonDown: () => (store.conn = 'daemon_down'),
    daemonUp: () => (store.conn = 'catching_up'),
    open: () => armStale(),
    error: () => {
      if (store.conn === 'live') store.conn = 'catching_up';
    },
  });
}

/** Test hook: loads a snapshot without a server. */
export function hydrateForTest(snapshot: Boot): void {
  store.hydrate(snapshot);
}
