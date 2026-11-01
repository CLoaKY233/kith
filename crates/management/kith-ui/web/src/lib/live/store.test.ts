import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { viewOf } from './core.svelte';
import { deviceKey, tintKey } from './keys';
import { bootAt, entry, fixture, Harness, NET, node, SELF, settle, tickAfter } from './testing';
import {
  BANNER_DELAY_MS,
  CATCHING_UP_QUIET_MS,
  DAEMON_POLL_MS,
  JOURNAL_CAP,
  PATH_HYSTERESIS_MS,
  RTT_TINT_TICKS,
  STALE_MS,
  TICK_MS,
  TINT_MS,
} from './timing';

const T0 = 1_800_000_000_000;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(T0);
});

afterEach(() => {
  vi.useRealTimers();
});

async function started(): Promise<Harness> {
  const h = new Harness();
  await h.store.start(bootAt(T0));
  return h;
}

const online = (seq: number, name: string, path: 'direct' | 'relay' | 'stealth' | null = 'direct') =>
  entry(seq, Date.now(), { kind: 'peer_online', network: NET, node: node(name), name, path, gateway: null });
const offline = (seq: number, name: string) =>
  entry(seq, Date.now(), { kind: 'peer_offline', network: NET, node: node(name), name });
const log = (seq: number) =>
  entry(seq, Date.now(), { kind: 'log', level: 'info', target: 'kith', message: `line ${seq}` });

describe('boot', () => {
  it('has complete data before the first frame, with the stream resuming from boot:seq', async () => {
    const h = await started();
    const live = viewOf(h.store);
    const seq = fixture.journal.at(-1)?.seq ?? 0;

    expect(live.ready).toBe(true);
    expect(live.conn).toBe('live');
    expect(Object.keys(live.peers[NET] ?? {})).toHaveLength(fixture.peers[NET]?.length ?? 0);
    const nas = live.numbers[deviceKey(NET, node('nas'))];
    expect(nas?.rttMs).toBe(3);
    expect(nas?.path).toBe('direct');
    expect(nas?.rx).toBeGreaterThan(0);
    expect(live.paths.get(deviceKey(NET, node('dev-box')))).toEqual({ path: 'relay', flapping: false });
    expect(live.seenAt[deviceKey(NET, node('tv'))]).toBe(T0 - 11_520_000);
    expect(live.changed.size).toBe(0);
    expect(h.stream.resume()).toBe(`${fixture.boot}:${seq}`);
  });

  it('polls /api/boot every 2 s behind the daemon gate and hydrates when it answers', async () => {
    const h = new Harness();
    h.fail('/api/boot', 'daemon_down', 503);
    await h.store.start(null);
    expect(h.store.gate).toBe('daemon_down');
    expect(h.store.ready).toBe(false);
    expect(h.streams).toHaveLength(0);

    h.answers.set('/api/boot', bootAt(T0));
    await vi.advanceTimersByTimeAsync(DAEMON_POLL_MS);
    expect(h.store.gate).toBeNull();
    expect(h.store.ready).toBe(true);
    expect(h.streams).toHaveLength(1);
  });
});

describe('commit', () => {
  it('applies a burst of entries together in the next frame, in seq order, once each', async () => {
    const h = await started();
    const seq = h.store.seq;
    const seen: number[] = [];
    h.store.onEntry((e) => seen.push(e.seq));

    h.stream.on.entry(offline(seq, 'nas'));
    h.stream.on.entry(offline(seq + 1, 'nas'));
    h.stream.on.entry(online(seq + 2, 'tv'));
    h.stream.on.entry(online(seq + 2, 'tv'));
    expect(h.store.peers[NET]?.[node('nas')]?.online).toBe(true);
    expect(h.frames).toHaveLength(1);

    h.frame();
    expect(h.store.peers[NET]?.[node('nas')]?.online).toBe(false);
    expect(h.store.peers[NET]?.[node('tv')]?.online).toBe(true);
    expect(h.store.seq).toBe(seq + 2);
    expect(seen).toEqual([seq + 1, seq + 2]);
    expect(h.stream.resume()).toBe(`${fixture.boot}:${seq + 2}`);
    expect(h.store.status?.networks[0]?.peers_online).toBe(fixture.status.networks[0]?.peers_online);
  });

  it('keeps the journal at its cap and counts every entry ever appended', async () => {
    const h = await started();
    const seq = h.store.seq;
    for (let i = 1; i <= JOURNAL_CAP + 5; i += 1) h.stream.on.entry(log(seq + i));
    h.frame();
    expect(h.store.journal).toHaveLength(JOURNAL_CAP);
    expect(h.store.journal[0]?.seq).toBe(seq + 6);
    expect(h.store.journalTotal).toBe(JOURNAL_CAP + 5);
  });

  it('changes numbers only on a tick, all together', async () => {
    const h = await started();
    const key = deviceKey(NET, node('nas'));
    const first = h.store.numbers;
    const boot = bootAt(T0).last_tick;
    if (!boot) throw new Error('fixture has a last tick');

    vi.advanceTimersByTime(TICK_MS);
    h.stream.on.tick(tickAfter(boot, TICK_MS, 4000, 2000));
    expect(h.store.numbers).toBe(first);
    h.frame();
    expect(h.store.numbers[key]?.rx).toBe(2000);
    expect(h.store.numbers[key]?.tx).toBe(1000);
    expect(h.store.seenAt[key]).toBe(T0 + TICK_MS);
    expect(h.store.asOfMs).toBe(T0 + TICK_MS);
  });
});

describe('connection', () => {
  it('goes stale 5 s after the last tick and shows it 3 s later; a tick brings it back', async () => {
    const h = await started();
    vi.advanceTimersByTime(STALE_MS);
    expect(h.store.conn).toBe('stale');
    expect(h.store.connShown).toBe('live');
    expect(h.stream.reconnects).toBe(1);
    vi.advanceTimersByTime(BANNER_DELAY_MS);
    expect(h.store.connShown).toBe('stale');

    const boot = bootAt(T0).last_tick;
    if (!boot) throw new Error('fixture has a last tick');
    h.stream.on.tick(tickAfter(boot, STALE_MS + BANNER_DELAY_MS, 0));
    expect(h.store.conn).toBe('live');
    expect(h.store.connShown).toBe('live');
  });

  it('shows Reconnecting only after 1 s, and the daemon banner only after 3 s', async () => {
    const h = await started();
    h.answers.set('/api/status', fixture.status);
    h.stream.on.error();
    await settle();
    expect(h.store.conn).toBe('catching_up');
    vi.advanceTimersByTime(CATCHING_UP_QUIET_MS - 1);
    expect(h.store.connShown).toBe('live');
    vi.advanceTimersByTime(1);
    expect(h.store.connShown).toBe('catching_up');

    h.stream.on.daemonDown();
    vi.advanceTimersByTime(BANNER_DELAY_MS - 1);
    expect(h.store.connShown).toBe('catching_up');
    vi.advanceTimersByTime(1);
    expect(h.store.connShown).toBe('daemon_down');
  });

  it('tells a lost kith-ui from a restarted one when the stream fails', async () => {
    const h = await started();
    h.fail('/api/status', 'internal', 0);
    h.stream.on.error();
    await settle();
    expect(h.store.conn).toBe('ui_gone');
    expect(h.store.connShown).toBe('ui_gone');

    h.fail('/api/status', 'unauthorized', 401);
    h.stream.on.error();
    await settle();
    expect(h.store.conn).toBe('unauth');
    expect(h.streams.every((s) => s.closed)).toBe(true);
  });

  it('refetches everything on reset and resumes from the new boot', async () => {
    const h = await started();
    const old = h.stream;
    const fresh = bootAt(T0, { boot: 'b2', seq: 3 });
    fresh.status.exit_node = null;
    h.answers.set('/api/boot', fresh);

    old.on.reset({ boot: 'b2', started_at_ms: T0 - 500 });
    await settle();
    expect(old.closed).toBe(true);
    expect(h.store.boot).toBe('b2');
    expect(h.store.restartedAtMs).toBe(T0 - 500);
    expect(h.store.changed.has(tintKey.exit)).toBe(true);
    expect(h.stream.resume()).toBe('b2:3');

    h.stream.on.entry(log(4));
    h.frame();
    expect(h.store.journal.at(-1)?.seq).toBe(4);
  });

  it('marks a gap, refetches, and applies entries that arrived meanwhile on top', async () => {
    const h = await started();
    const seq = h.store.seq;
    const resynced = bootAt(T0, { seq: seq + 10 });
    const tv = resynced.peers[NET]?.find((p) => p.node === node('tv'));
    if (tv) tv.online = true;
    let answer: (value: unknown) => void = () => undefined;
    h.answers.set('/api/boot', () => new Promise((resolve) => (answer = resolve)));

    h.stream.on.entry(log(seq + 1));
    h.stream.on.gap({ missed_from: seq + 2, missed_to: seq + 9 });
    h.stream.on.entry(offline(seq + 10, 'nas'));
    h.stream.on.entry(offline(seq + 11, 'vps'));
    h.frame();
    expect(h.store.peers[NET]?.[node('vps')]?.online).toBe(true);

    answer(resynced);
    await settle();
    expect(h.store.peers[NET]?.[node('tv')]?.online).toBe(true);
    expect(h.store.peers[NET]?.[node('nas')]?.online).toBe(true);
    expect(h.store.peers[NET]?.[node('vps')]?.online).toBe(false);
    expect(h.store.seq).toBe(seq + 11);
    expect(h.store.gaps).toEqual([
      { from: seq + 2, to: seq + 9, fromMs: T0, toMs: T0, position: 1 },
    ]);
    expect(h.store.changed.has(tintKey.device(NET, node('tv')))).toBe(true);
  });
});

describe('highlights', () => {
  it('tints a changed device for 4 s, and holds hidden-tab changes until the tab returns', async () => {
    const h = await started();
    const seq = h.store.seq;
    const nas = tintKey.device(NET, node('nas'));
    h.stream.on.entry(offline(seq + 1, 'nas'));
    h.frame();
    expect(h.store.changed.has(nas)).toBe(true);
    vi.advanceTimersByTime(TINT_MS);
    expect(h.store.changed.has(nas)).toBe(false);

    h.setVisible(false);
    h.stream.on.entry(online(seq + 2, 'nas'));
    h.stream.on.entry(online(seq + 3, 'tv'));
    vi.advanceTimersByTime(0);
    expect(h.store.peers[NET]?.[node('tv')]?.online).toBe(true);
    expect(h.store.unseen).toBe(2);
    expect(h.store.changed.size).toBe(0);

    h.setVisible(true);
    expect(h.store.unseen).toBe(0);
    expect(h.store.changed.has(nas)).toBe(true);
    expect(h.store.changed.has(tintKey.device(NET, node('tv')))).toBe(true);
  });

  it('tints latency only when it more than doubles and stays so for 3 ticks', async () => {
    const h = await started();
    const boot = bootAt(T0).last_tick;
    if (!boot) throw new Error('fixture has a last tick');
    const rtt = tintKey.rtt(NET, node('nas'));
    let tick = boot;
    const send = (ms: number): void => {
      tick = tickAfter(tick, TICK_MS, 0);
      tick.links = tick.links.map((l) => (l.from === SELF && l.to === node('nas') ? { ...l, rtt_ms: ms } : l));
      h.stream.on.tick(tick);
      h.frame();
    };
    send(9);
    send(2);
    for (let i = 1; i < RTT_TINT_TICKS; i += 1) send(9);
    expect(h.store.changed.has(rtt)).toBe(false);
    send(9);
    expect(h.store.changed.has(rtt)).toBe(true);
    expect(h.store.changed.has(tintKey.device(NET, node('nas')))).toBe(false);
  });

  it('shows a path style only after it held for 5 s, then tints it', async () => {
    const h = await started();
    const seq = h.store.seq;
    const key = deviceKey(NET, node('nas'));
    const change = (s: number, from: 'direct' | 'relay', to: 'direct' | 'relay') =>
      entry(s, Date.now(), { kind: 'path_changed', network: NET, node: node('nas'), name: 'nas', from, to });

    h.stream.on.entry(change(seq + 1, 'direct', 'relay'));
    h.frame();
    expect(h.store.peers[NET]?.[node('nas')]?.path).toBe('relay');
    expect(h.store.paths.shown.get(key)?.path).toBe('direct');
    vi.advanceTimersByTime(PATH_HYSTERESIS_MS - 1);
    expect(h.store.paths.shown.get(key)?.path).toBe('direct');
    vi.advanceTimersByTime(1);
    expect(h.store.paths.shown.get(key)?.path).toBe('relay');
    expect(h.store.changed.has(tintKey.device(NET, node('nas')))).toBe(true);
  });
});
