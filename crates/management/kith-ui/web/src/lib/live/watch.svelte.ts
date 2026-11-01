// Reacts to what the store already holds, so the stream code needs no hooks: each new
// journal entry goes to the action runner (seen), the notification rules (§7.12) and the
// announcer (§11.1); connection changes drive the daemon lost/recovered announcements and
// the daemon-stopped notification.

import { untrack } from 'svelte';
import { announce } from '../announce';
import { copy } from '../copy';
import type { Entry, StatusView } from '../types.gen';
import type { ConnState, TrafficColumns } from '../types';
import { daemonNotice, noticeFor, type Notice, type NoticeContext } from './notify';
import type { Runner } from './runner';
import {
  BANNER_DELAY_MS,
  BLOCKED_ANNOUNCE_MS,
  DAEMON_STOPPED_NOTIFY_MS,
  IDLE_BYTES_PER_SEC,
  NOTIFY_REPLAY_CUTOFF_MS,
  RECENTLY_USED_MS,
  SECOND,
} from './timing';

/** The store fields the watcher reads. */
export interface WatchHost {
  readonly boot: string;
  readonly journal: readonly Entry[];
  readonly conn: ConnState;
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

/** Follows the journal and the connection; the store calls `entries` and `conn` from effects. */
export class Watcher {
  private readonly host: WatchHost;
  private readonly runner: Runner;
  private readonly notices: NoticeSink;
  private readonly ctx: NoticeContext;
  private boot = '';
  private lastSeq = -1;
  private lastConn: ConnState | null = null;
  private lostTimer: ReturnType<typeof setTimeout> | null = null;
  private stoppedTimer: ReturnType<typeof setTimeout> | null = null;
  private lostAnnounced = false;

  constructor(host: WatchHost, runner: Runner, notices: NoticeSink) {
    this.host = host;
    this.runner = runner;
    this.notices = notices;
    this.ctx = noticeContext(host);
  }

  /** Handles every journal entry not seen yet, oldest first. */
  entries(): void {
    const { journal, boot } = this.host;
    if (boot !== this.boot) {
      this.boot = boot;
      this.lastSeq = -1;
    }
    const fresh: Entry[] = [];
    for (let i = journal.length - 1; i >= 0; i -= 1) {
      const entry = journal[i];
      if (!entry || entry.seq <= this.lastSeq) break;
      // After a reset the ring still ends with the old boot's entries: stop there.
      const newer = fresh[fresh.length - 1];
      if (newer && entry.seq >= newer.seq) break;
      fresh.push(entry);
    }
    if (fresh.length === 0) return;
    fresh.reverse();
    this.lastSeq = fresh[fresh.length - 1]?.seq ?? this.lastSeq;
    const now = this.host.now();
    for (const entry of fresh) {
      this.runner.observe(entry);
      if (now - entry.at_ms > NOTIFY_REPLAY_CUTOFF_MS) continue;
      const notice = noticeFor(entry, this.ctx);
      if (notice) this.notices.deliver(notice);
      announceEntry(entry, this.ctx);
    }
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

  /** Stops the timers; for tests and teardown. */
  clearTimers(): void {
    if (this.lostTimer !== null) clearTimeout(this.lostTimer);
    if (this.stoppedTimer !== null) clearTimeout(this.stoppedTimer);
    this.lostTimer = null;
    this.stoppedTimer = null;
  }
}

/** Runs a Watcher off the store's reactive state. Returns the function that stops it. */
export function watch(host: WatchHost, runner: Runner, notices: NoticeSink): () => void {
  const watcher = new Watcher(host, runner, notices);
  const stop = $effect.root(() => {
    $effect(() => {
      void host.journal;
      void host.boot;
      untrack(() => watcher.entries());
    });
    $effect(() => {
      void host.conn;
      untrack(() => watcher.conn());
    });
  });
  return () => {
    watcher.clearTimers();
    stop();
  };
}
