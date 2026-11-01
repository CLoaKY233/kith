// Follows the store: each new journal entry (from the store's onEntry) goes to the action
// runner (seen), the notification rules (§7.12) and the announcer (§11.1); connection
// changes drive the daemon lost/recovered announcements and the daemon-stopped notification.

import { untrack } from 'svelte';
import { announce } from '../announce';
import { copy } from '../copy';
import type { Entry, StatusView } from '../types.gen';
import type { ConnState, GateReason, TrafficColumns } from '../types';
import { daemonNotice, noticeFor, type Notice, type NoticeContext } from './notify';
import type { Runner } from './runner';
import {
  BANNER_DELAY_MS,
  BLOCKED_ANNOUNCE_MS,
  DAEMON_STOPPED_NOTIFY_MS,
  IDLE_BYTES_PER_SEC,
  MOTION,
  NOTIFY_REPLAY_CUTOFF_MS,
  RECENTLY_USED_MS,
  SECOND,
} from './timing';

/** The store fields the watcher reads. */
export interface WatchHost {
  readonly conn: ConnState;
  readonly gate?: GateReason | null;
  readonly status: StatusView | null;
  readonly traffic: ReadonlyMap<string, TrafficColumns>;
  now(): number;
}

/** Where notices go; the Notifications class in the page, a list in tests. */
export interface NoticeSink {
  deliver(notice: Notice): void;
}

/** §7.12 context over the store. */
export function noticeContext(host: WatchHost): NoticeContext {
  return {
    networkName: (id) => host.status?.networks.find((n) => n.id === id)?.name ?? null,
    isAdmin: (id) => host.status?.networks.find((n) => n.id === id)?.admin ?? false,
    exitNode: () => host.status?.exit_node ?? null,
    usedRecently: (network, node, atMs) => {
      const columns = host.traffic.get(`${network}/${node}`);
      if (!columns) return false;
      for (let i = columns.length - 1; i > 0; i -= 1) {
        const at = columns.at[i] ?? 0;
        const before = columns.at[i - 1] ?? 0;
        if (at < atMs - RECENTLY_USED_MS) break;
        if (at > atMs || at <= before) continue;
        const moved =
          Math.max(0, (columns.rx[i] ?? 0) - (columns.rx[i - 1] ?? 0)) +
          Math.max(0, (columns.tx[i] ?? 0) - (columns.tx[i - 1] ?? 0));
        if (moved / ((at - before) / SECOND) >= IDLE_BYTES_PER_SEC) return true;
      }
      return false;
    },
  };
}

/** §11.1: what an entry announces, if anything. Join requests reach admins only. */
function announceEntry(entry: Entry, ctx: NoticeContext): void {
  if (entry.kind === 'join_requested' && ctx.isAdmin(entry.network)) {
    announce(copy.overview.joinOne(entry.name, ctx.networkName(entry.network) ?? entry.network));
  } else if (entry.kind === 'blocked') {
    const port = entry.port === null ? '' : copy.blocked.port(entry.port, null);
    announce(copy.entry.blocked(entry.from_name, entry.to_name, port, 1), {
      key: `blocked:${entry.network}:${entry.from}:${entry.to}:${entry.protocol}:${entry.port ?? ''}`,
      every: BLOCKED_ANNOUNCE_MS,
    });
  }
}

/** Follows the journal and the connection; `watch` calls `entry` and `conn`. */
export class Watcher {
  private readonly host: WatchHost;
  private readonly runner: Runner;
  private readonly notices: NoticeSink;
  private readonly ctx: NoticeContext;
  private lastConn: ConnState | null = null;
  private lostTimer: ReturnType<typeof setTimeout> | null = null;
  private stoppedTimer: ReturnType<typeof setTimeout> | null = null;
  private lostAnnounced = false;
  private lastGate: GateReason | null = null;
  private readonly focusMain: () => void;

  constructor(host: WatchHost, runner: Runner, notices: NoticeSink, focusMain: () => void = () => {}) {
    this.host = host;
    this.focusMain = focusMain;
    this.runner = runner;
    this.notices = notices;
    this.ctx = noticeContext(host);
  }

  /** Handles one new journal entry: marks actions seen, then notifies and announces it
   * unless it is a replay older than the cutoff. */
  entry(entry: Entry): void {
    this.runner.observe(entry);
    if (this.host.now() - entry.at_ms > NOTIFY_REPLAY_CUTOFF_MS) return;
    const notice = noticeFor(entry, this.ctx);
    if (notice) this.notices.deliver(notice);
    announceEntry(entry, this.ctx);
  }

  /** Handles a connection change: daemon lost and recovered (assertive), daemon stopped. */
  conn(): void {
    const conn = this.host.conn;
    if (conn === this.lastConn) return;
    const was = this.lastConn;
    this.lastConn = conn;
    if (conn === 'daemon_down') {
      this.lostTimer = setTimeout(() => {
        this.lostTimer = null;
        this.lostAnnounced = true;
        announce(copy.conn.daemonLost, 'assertive');
      }, BANNER_DELAY_MS);
      this.stoppedTimer = setTimeout(() => {
        this.stoppedTimer = null;
        this.notices.deliver(daemonNotice(this.host.now()));
      }, DAEMON_STOPPED_NOTIFY_MS);
      return;
    }
    if (was === 'daemon_down') this.clearTimers();
    if (conn === 'live' && this.lostAnnounced) {
      this.lostAnnounced = false;
      announce(copy.conn.daemonBack, 'assertive');
    }
  }

  /** Handles a gate change: leaving the daemon-down or group gate is the daemon recovered
   * (§11.1), so it is announced and focus moves to the new main. */
  gate(): void {
    const gate = this.host.gate ?? null;
    const was = this.lastGate;
    this.lastGate = gate;
    if (gate !== null || (was !== 'daemon_down' && was !== 'not_in_group')) return;
    this.lostAnnounced = false;
    announce(copy.conn.daemonBack, 'assertive');
    this.focusMain();
  }

  /** Stops the timers; for tests and teardown. */
  clearTimers(): void {
    if (this.lostTimer !== null) clearTimeout(this.lostTimer);
    if (this.stoppedTimer !== null) clearTimeout(this.stoppedTimer);
    this.lostTimer = null;
    this.stoppedTimer = null;
  }
}

/** Moves focus to the app's main once the gate has faded out. */
function focusContent(): void {
  if (typeof document === 'undefined') return;
  setTimeout(() => {
    const mains = document.querySelectorAll<HTMLElement>('#content');
    mains[mains.length - 1]?.focus();
  }, MOTION.dialog);
}

/** Runs a Watcher off the store: `subscribe` delivers each new journal entry once, and the
 * connection is followed reactively. Returns the function that stops it. */
export function watch(
  host: WatchHost,
  runner: Runner,
  notices: NoticeSink,
  subscribe: (listener: (entry: Entry) => void) => () => void,
): () => void {
  const watcher = new Watcher(host, runner, notices, focusContent);
  const unsubscribe = subscribe((entry) => watcher.entry(entry));
  const stop = $effect.root(() => {
    $effect(() => {
      void host.conn;
      untrack(() => watcher.conn());
    });
    $effect(() => {
      void host.gate;
      untrack(() => watcher.gate());
    });
  });
  return () => {
    unsubscribe();
    watcher.clearTimers();
    stop();
  };
}
