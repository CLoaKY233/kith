import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Entry } from '../types.gen';
import { backoff, connect, type StreamEnv, type StreamHandlers } from './stream';
import { RECONNECT_BACKOFF_MS, RECONNECT_JITTER } from './timing';

class FakeSource extends EventTarget {
  static all: FakeSource[] = [];
  closed = false;
  constructor(readonly url: string) {
    super();
    FakeSource.all.push(this);
  }
  close(): void {
    this.closed = true;
  }
  emit(name: string, data?: unknown): void {
    this.dispatchEvent(
      data === undefined ? new Event(name) : new MessageEvent(name, { data: JSON.stringify(data) }),
    );
  }
}

class FakeDocument extends EventTarget {
  visibilityState: DocumentVisibilityState = 'visible';
}

function setup(): { env: StreamEnv; doc: FakeDocument; on: StreamHandlers; calls: string[] } {
  const doc = new FakeDocument();
  const calls: string[] = [];
  const on: StreamHandlers = {
    entry: (e) => calls.push(`entry ${e.seq}`),
    tick: (t) => calls.push(`tick ${t.at_ms}`),
    reset: (r) => calls.push(`reset ${r.boot}`),
    gap: (g) => calls.push(`gap ${g.missed_from}-${g.missed_to}`),
    daemonDown: () => calls.push('daemon_down'),
    daemonUp: () => calls.push('daemon_up'),
    open: () => calls.push('open'),
    error: () => calls.push('error'),
  };
  const env: StreamEnv = {
    EventSource: FakeSource as unknown as StreamEnv['EventSource'],
    document: doc,
    random: () => 1,
  };
  return { env, doc, on, calls };
}

const last = (): FakeSource => {
  const source = FakeSource.all.at(-1);
  if (!source) throw new Error('no source');
  return source;
};

beforeEach(() => {
  vi.useFakeTimers();
  FakeSource.all = [];
});

afterEach(() => {
  vi.useRealTimers();
});

describe('stream', () => {
  it('waits 0.5, 1, 2, then 4 s with jitter between retries, resuming from the latest point', () => {
    const { env, on, calls } = setup();
    let seq = 7;
    connect(() => `b1:${seq}`, on, env);
    expect(last().url).toBe('/api/events?last=b1%3A7');

    const waits: number[] = [];
    for (let i = 0; i < 5; i += 1) {
      const before = FakeSource.all.length;
      last().emit('error');
      let waited = 0;
      while (FakeSource.all.length === before) {
        vi.advanceTimersByTime(10);
        waited += 10;
      }
      waits.push(waited);
    }
    const jittered = (RECONNECT_BACKOFF_MS.at(-1) ?? 0) * (1 + RECONNECT_JITTER);
    expect(waits).toEqual([...RECONNECT_BACKOFF_MS.slice(0, -1), jittered, jittered]);
    expect(calls.filter((c) => c === 'error')).toHaveLength(5);

    seq = 9;
    last().emit('open');
    last().emit('error');
    vi.advanceTimersByTime(RECONNECT_BACKOFF_MS[0]);
    expect(last().url).toBe('/api/events?last=b1%3A9');
  });

  it('jitters the last step both ways', () => {
    const top = RECONNECT_BACKOFF_MS.at(-1) ?? 0;
    const late = RECONNECT_BACKOFF_MS.length;
    expect(backoff(late, () => 0)).toBe(top * (1 - RECONNECT_JITTER));
    expect(backoff(late, () => 1)).toBe(top * (1 + RECONNECT_JITTER));
    expect(backoff(0, () => 0)).toBe(RECONNECT_BACKOFF_MS[0]);
  });

  it('hands over parsed events, skips bad ones and ignores a source it dropped', () => {
    const { env, on, calls } = setup();
    const control = connect(() => 'b1:0', on, env);
    const first = last();
    first.emit('open');
    first.emit('entry', { seq: 1, at_ms: 5, kind: 'warning', message: 'x' } satisfies Entry);
    first.dispatchEvent(new MessageEvent('entry', { data: '{not json' }));
    first.emit('tick', { at_ms: 2000, traffic: [], links: [] });
    first.emit('gap', { missed_from: 2, missed_to: 4 });
    first.emit('daemon_down');
    first.emit('daemon_up');
    first.emit('reset', { boot: 'b2', started_at_ms: 1 });

    control.reconnect();
    expect(first.closed).toBe(true);
    first.emit('entry', { seq: 2, at_ms: 6, kind: 'warning', message: 'y' } satisfies Entry);
    expect(calls).toEqual([
      'open',
      'entry 1',
      'tick 2000',
      'gap 2-4',
      'daemon_down',
      'daemon_up',
      'reset b2',
    ]);
  });

  it('retries at once when the tab becomes visible, and stops for good when closed', () => {
    const { env, doc, on } = setup();
    const control = connect(() => 'b1:0', on, env);
    last().emit('error');
    expect(FakeSource.all).toHaveLength(1);
    doc.visibilityState = 'visible';
    doc.dispatchEvent(new Event('visibilitychange'));
    expect(FakeSource.all).toHaveLength(2);

    last().emit('error');
    control.close();
    vi.advanceTimersByTime(60_000);
    doc.dispatchEvent(new Event('visibilitychange'));
    expect(FakeSource.all).toHaveLength(2);
  });
});
