import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError } from '../api';
import { copy } from '../copy';
import type { Entry, Outcome } from '../types.gen';
import type { ConnState } from '../types';
import { progressText, Runner } from './runner';
import { SEEN_TIMEOUT_MS } from './timing';

const NET = 'net1';
const joined = (seq: number, node: string): Entry => ({
  seq,
  at_ms: 0,
  kind: 'device_joined',
  network: NET,
  node,
  name: node,
  address: '100.64.0.9',
});

/** A promise the test settles by hand, standing in for the server's answer. */
function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  let reject: (cause: unknown) => void = () => undefined;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const accepted = (op: string | null = null): Outcome => ({ state: 'accepted', op });
const flush = () => vi.advanceTimersByTimeAsync(0);

let host: { conn: ConnState };
let runner: Runner;

beforeEach(() => {
  vi.useFakeTimers();
  host = { conn: 'live' };
  runner = new Runner(host);
});

afterEach(() => vi.useRealTimers());

describe('action runner (spec §8.6)', () => {
  it('moves a ledger op from pending to accepted to seen when its journal entry arrives', async () => {
    const answer = deferred<Outcome>();
    const handle = runner.run({
      ledger: true,
      run: () => answer.promise,
      match: (e) => e.kind === 'device_joined' && e.node === 'dev-box',
    });
    expect(handle.state).toBe('pending');
    expect(progressText(handle)).toBe(copy.action.signing);

    answer.resolve(accepted());
    await handle.done;
    expect(handle.state).toBe('accepted');
    expect(progressText(handle)).toBe(copy.action.savedHere);

    runner.observe(joined(7, 'someone-else'));
    expect(handle.state).toBe('accepted');
    runner.observe(joined(8, 'dev-box'));
    expect(handle.state).toBe('seen');
    expect(handle.seen).toBeNull();
    expect(progressText(handle)).toBe(copy.action.savedHere);
  });

  it('counts a journal entry that beats the 2xx back', async () => {
    const answer = deferred<Outcome>();
    const handle = runner.run({ ledger: true, run: () => answer.promise, match: () => true });
    await flush();
    runner.observe(joined(3, 'dev-box'));
    answer.resolve(accepted());
    await handle.done;
    expect(handle.state).toBe('seen');
  });

  it('turns slow after the seen timeout, then spreads with acks', async () => {
    const handle = runner.run({ ledger: true, run: async () => accepted('op-1') });
    await handle.done;
    vi.advanceTimersByTime(SEEN_TIMEOUT_MS);
    expect(handle.slow).toBe(true);
    expect(progressText(handle)).toBe(copy.action.savedWaiting);

    runner.ack('op-1', 2, 3);
    expect(handle.state).toBe('seen');
    expect(handle.slow).toBe(false);
    expect(progressText(handle)).toBe('Signed. Spreading · 2 of 3 devices');
    runner.ack('op-1', 3, 3);
    expect(handle.seen).toEqual({ by: 3, of: 3 });
  });

  it('keeps counting acks after the journal entry, and takes an ack that beat the 2xx', async () => {
    runner.ack('op-2', 1, 4);
    const handle = runner.run({ ledger: true, run: async () => accepted('op-2'), match: () => true });
    await handle.done;
    expect(handle.seen).toEqual({ by: 1, of: 4 });
    runner.observe(joined(9, 'dev-box'));
    runner.ack('op-2', 4, 4);
    expect(handle.seen).toEqual({ by: 4, of: 4 });
  });

  it('never waits for a seen that cannot come (no acks, no journal entry)', async () => {
    const handle = runner.run({ ledger: true, run: async () => accepted(null) });
    await handle.done;
    vi.advanceTimersByTime(SEEN_TIMEOUT_MS * 2);
    expect(handle.slow).toBe(false);
    expect(progressText(handle)).toBe(copy.action.savedHere);
  });

  it("shows a 409 as the other device's result and loads the latest", async () => {
    const refresh = vi.fn(async () => undefined);
    const conflict = { resolved_by: 'laptop', resolution: 'approved' as const, at_ms: 1, op: null };
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify({ code: 'conflict', message: '', conflict }), { status: 409 })),
    );
    const handle = runner.run({ ledger: true, run: () => api.post<Outcome>('/x'), refresh });
    await handle.done;
    vi.unstubAllGlobals();
    expect(handle.state).toBe('conflict');
    expect(handle.error).toBeNull();
    expect(handle.conflict).toEqual(conflict);
    expect(refresh).toHaveBeenCalledOnce();
  });

  it('queues nothing while stale: changes fail at once, reads still go out', async () => {
    host.conn = 'stale';
    const run = vi.fn(async () => accepted());
    const change = runner.run({ ledger: true, run });
    await change.done;
    expect(run).not.toHaveBeenCalled();
    expect(change.state).toBe('failed');
    expect(progressText(change)).toBe(copy.conn.staleActions);

    const read = runner.run({ read: true, run: async () => [1, 2] });
    await read.done;
    expect(read.result).toEqual([1, 2]);
  });

  it('waits for the OS on tier-3 actions, and a 401 marks the session gone', async () => {
    const answer = deferred<Outcome>();
    const handle = runner.run({ ledger: true, tier3: true, run: () => answer.promise });
    expect(handle.state).toBe('awaiting_os_auth');
    expect(progressText(handle)).toBe(copy.tier3.waiting);
    answer.reject(new ApiError('unauthorized', 401, '', null));
    await handle.done;
    expect(handle.state).toBe('failed');
    expect(host.conn).toBe('unauth');
  });

  it('lets only the newest call of a latest key count', async () => {
    const call = (name: string) =>
      runner.run({
        read: true,
        latest: 'validate:home',
        run: (signal) =>
          new Promise<string>((resolve, reject) => {
            signal.addEventListener('abort', () =>
              reject(Object.assign(new Error('aborted'), { name: 'AbortError' })),
            );
            setTimeout(() => resolve(name), 10);
          }),
      });
    const first = call('nas');
    const second = call('nas-2');
    await vi.advanceTimersByTimeAsync(10);
    await Promise.all([first.done, second.done]);
    expect(first.state).toBe('idle');
    expect(second.result).toBe('nas-2');
  });
});
