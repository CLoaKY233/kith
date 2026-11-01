import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { attachAnnouncer, resetAnnouncer } from '../announce';
import { copy } from '../copy';
import type { Entry, StatusView } from '../types.gen';
import type { ConnState, TrafficColumns } from '../types';
import { Notifications } from './notifications.svelte';
import { noticeFor, type Notice, type NoticeContext, type NotifyEnv, type Permission } from './notify';
import { Runner } from './runner';
import {
  BANNER_DELAY_MS,
  DAEMON_STOPPED_NOTIFY_MS,
  NOTIFY_PROMPT_SNOOZE_MS,
  NOTIFY_REPLAY_CUTOFF_MS,
} from './timing';
import { noticeContext, Watcher, type WatchHost } from './watch.svelte';

const NET = 'net1';
const NOW = 1_800_000_000_000;

const entry = (seq: number, at: number, body: Record<string, unknown>): Entry =>
  ({ seq, at_ms: at, network: NET, ...body }) as Entry;
const joinRequest = (seq: number, at = NOW, node = 'n-pixel'): Entry =>
  entry(seq, at, { kind: 'join_requested', node, name: 'pixel-8' });
const offline = (seq: number, node: string, name: string): Entry =>
  entry(seq, NOW, { kind: 'peer_offline', node, name });

const ctx = (over: Partial<NoticeContext> = {}): NoticeContext => ({
  networkName: () => 'home',
  isAdmin: () => true,
  exitNode: () => 'home/vps',
  usedRecently: () => false,
  ...over,
});

/** A browser double: hidden tab, notifications allowed, everything recorded. */
class FakeEnv implements NotifyEnv {
  clock = NOW;
  tab: 'hidden' | 'blurred' | 'focused' = 'hidden';
  granted: Permission = 'granted';
  stored = new Map<string, string>();
  shown: { title: string; tag: string; closed: boolean }[] = [];
  opened: Notice['need'][] = [];
  now = () => this.clock;
  hidden = () => this.tab === 'hidden';
  focused = () => this.tab === 'focused';
  permission = () => this.granted;
  requestPermission = async () => this.granted;
  show = (notice: Pick<Notice, 'title' | 'body' | 'tag'>, onClick: () => void) => {
    const record = { title: notice.title, tag: notice.tag, closed: false };
    this.shown.push(record);
    this.clicks.set(notice.tag, () => {
      record.closed = true;
      onClick();
    });
    return { close: () => (record.closed = true) };
  };
  clicks = new Map<string, () => void>();
  open = (need: Notice['need']) => void this.opened.push(need);
  load = (key: string) => this.stored.get(key) ?? null;
  save = (key: string, value: string) => void this.stored.set(key, value);
}

describe('notification rules (spec §7.12)', () => {
  it('notifies admins of join requests, opening the page at ?need=', () => {
    const notice = noticeFor(joinRequest(1), ctx());
    expect(notice?.title).toBe('"pixel-8" wants to join home');
    expect(notice?.body).toBe('Open kith to check the code and approve.');
    expect(notice?.need).toEqual({ network: NET, id: `join:${NET}:n-pixel` });
    expect(noticeFor(joinRequest(1), ctx({ isAdmin: () => false }))).toBeNull();
  });

  it('notifies the exit or a recently-used device going offline, nothing else', () => {
    expect(noticeFor(offline(1, 'n-vps', 'vps'), ctx())?.title).toBe('Your exit, vps, is offline');
    expect(noticeFor(offline(1, 'n-tv', 'tv'), ctx())).toBeNull();
    expect(noticeFor(offline(1, 'n-tv', 'tv'), ctx({ usedRecently: () => true }))?.tag).toBe(
      `kith:device_offline:${NET}:n-tv`,
    );
    const quiet: Entry[] = [
      entry(1, NOW, { kind: 'peer_online', node: 'n', name: 'n', path: null, gateway: null }),
      entry(2, NOW, { kind: 'path_changed', node: 'n', name: 'n', from: null, to: null }),
      { seq: 3, at_ms: NOW, kind: 'log', level: 'error', target: 't', message: 'm' },
    ];
    for (const e of quiet) expect(noticeFor(e, ctx({ usedRecently: () => true }))).toBeNull();
  });

  it('calls a device recently used only when it moved data in the window', () => {
    const columns: TrafficColumns = {
      at: Float64Array.from([NOW - 60_000, NOW - 58_000, NOW - 56_000]),
      rx: Float64Array.from([0, 0, 50_000]),
      tx: Float64Array.from([0, 0, 0]),
      length: 3,
    };
    const host = { status: null, traffic: new Map([[`${NET}/n-tv`, columns]]) } as unknown as WatchHost;
    expect(noticeContext(host).usedRecently(NET, 'n-tv', NOW)).toBe(true);
    expect(noticeContext(host).usedRecently(NET, 'n-tv', NOW - 57_000)).toBe(false);
  });
});

describe('notification delivery and opt-in (spec §7.12)', () => {
  let env: FakeEnv;
  let notes: Notifications;
  const join = (node: string, at = NOW) => noticeFor(joinRequest(1, at, node), ctx()) as Notice;

  beforeEach(() => {
    env = new FakeEnv();
    notes = new Notifications(env);
  });

  it('is off by default and offers the Activity card after a missed event in a hidden tab', () => {
    expect(notes.on).toBe(false);
    expect(notes.prompt).toBe(false);
    notes.deliver(join('a'));
    expect(env.shown).toHaveLength(0);
    expect(notes.prompt).toBe(true);

    notes.notNow();
    expect(notes.prompt).toBe(false);
    notes.deliver(join('b'));
    expect(notes.prompt).toBe(false);
    env.clock += NOTIFY_PROMPT_SNOOZE_MS;
    expect(notes.prompt).toBe(true);
  });

  it('remembers the choice, and stays off when the browser refuses', async () => {
    expect(await notes.turnOn()).toBe(true);
    expect(new Notifications(env).on).toBe(true);
    env.granted = 'denied';
    const refused = new Notifications(env);
    expect(await refused.setEnabled(true)).toBe(false);
    expect(refused.prompt).toBe(false);
  });

  it('never notifies while the tab is focused, or for entries older than 30 s', async () => {
    await notes.turnOn();
    env.tab = 'focused';
    notes.deliver(join('a'));
    env.tab = 'blurred';
    notes.deliver(join('b', NOW - NOTIFY_REPLAY_CUTOFF_MS - 1));
    expect(env.shown).toHaveLength(0);
    notes.deliver(join('c'));
    expect(env.shown.map((s) => s.title)).toEqual(['"pixel-8" wants to join home']);
  });

  it('coalesces more than 3 in 30 s into one, and a click opens the request', async () => {
    await notes.turnOn();
    for (const node of ['a', 'b', 'c']) notes.deliver(join(node));
    env.clicks.get(`kith:join:${NET}:a`)?.();
    expect(env.opened).toEqual([{ network: NET, id: `join:${NET}:a` }]);
    notes.deliver(join('d'));
    notes.deliver(join('e'));
    const open = env.shown.filter((s) => !s.closed);
    expect(open).toHaveLength(1);
    expect(open[0]?.title).toBe(copy.notify.coalesced(5));
  });
});

describe('watcher (spec §7.12, §11.1)', () => {
  let said: string[];
  let host: {
    boot: string;
    journal: Entry[];
    conn: ConnState;
    status: StatusView | null;
    traffic: Map<string, TrafficColumns>;
    now: () => number;
  };
  let delivered: Notice[];
  let watcher: Watcher;

  beforeEach(() => {
    vi.useFakeTimers();
    said = [];
    attachAnnouncer((_, message) => said.push(message));
    delivered = [];
    const status = {
      networks: [{ id: NET, name: 'home', admin: true }],
      exit_node: null,
    } as unknown as StatusView;
    host = { boot: 'b1', journal: [], conn: 'live', status, traffic: new Map(), now: () => NOW };
    watcher = new Watcher(host, new Runner(host), { deliver: (n) => delivered.push(n) });
  });

  afterEach(() => {
    watcher.clearTimers();
    resetAnnouncer();
    vi.useRealTimers();
  });

  it('handles each new entry once, skips stale replays, and restarts after a reset', () => {
    host.journal = [joinRequest(5, NOW - NOTIFY_REPLAY_CUTOFF_MS - 1, 'old'), joinRequest(6)];
    watcher.entries();
    watcher.entries();
    expect(delivered.map((n) => n.tag)).toEqual([`kith:join:${NET}:n-pixel`]);
    expect(said).toEqual(['pixel-8 is asking to join home.']);

    host.boot = 'b2';
    host.journal = [...host.journal, joinRequest(1, NOW, 'after-reset')];
    watcher.entries();
    expect(delivered.map((n) => n.tag)).toEqual([
      `kith:join:${NET}:n-pixel`,
      `kith:join:${NET}:after-reset`,
    ]);
  });

  it('announces the daemon lost after the banner delay and back once live, and notifies after 10 s', () => {
    watcher.conn();
    host.conn = 'daemon_down';
    watcher.conn();
    vi.advanceTimersByTime(BANNER_DELAY_MS);
    expect(said).toEqual([copy.conn.daemonLost]);
    vi.advanceTimersByTime(DAEMON_STOPPED_NOTIFY_MS - BANNER_DELAY_MS);
    expect(delivered.map((n) => n.title)).toEqual([copy.notify.daemonStopped]);
    host.conn = 'catching_up';
    watcher.conn();
    host.conn = 'live';
    watcher.conn();
    expect(said).toEqual([copy.conn.daemonLost, copy.conn.daemonBack]);
  });

  it('says nothing for a daemon blip shorter than the banner delay', () => {
    watcher.conn();
    host.conn = 'daemon_down';
    watcher.conn();
    vi.advanceTimersByTime(BANNER_DELAY_MS - 1);
    host.conn = 'live';
    watcher.conn();
    vi.advanceTimersByTime(DAEMON_STOPPED_NOTIFY_MS);
    expect(said).toEqual([]);
    expect(delivered).toEqual([]);
  });
});
