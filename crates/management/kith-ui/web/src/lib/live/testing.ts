// Fakes for the live layer's unit tests: the office fixture as a boot snapshot, a store
// with scripted deps, and an EventSource the test drives. Imported only by *.test.ts.

import { readFileSync } from 'node:fs';
import { ApiError } from '../api';
import type { ApiErrorCode, Boot, Entry, Fixture, Tick } from '../types.gen';
import { Store, type Deps } from './core.svelte';
import type { StreamControl, StreamHandlers } from './stream';

export const fixture = JSON.parse(
  readFileSync(new URL('../../../fixtures/office.json', import.meta.url), 'utf8'),
) as Fixture;

export const NET = fixture.status.networks[0]?.id ?? '';
export const SELF = fixture.status.node ?? '';

/** A device of the fixture, by name. */
export function node(name: string): string {
  const peer = fixture.peers[NET]?.find((p) => p.name === name);
  if (!peer) throw new Error(`no device ${name} in the fixture`);
  return peer.node;
}

/** The fixture as the server would inline it, timed `now`. */
export function bootAt(now: number, overrides: Partial<Boot> = {}): Boot {
  const shift = now - fixture.recorded_at_ms;
  const traffic = structuredClone(fixture.traffic).map((series) => ({
    ...series,
    samples: series.samples.slice(-150).map((s) => ({ ...s, at_ms: s.at_ms + shift })),
  }));
  return {
    boot: fixture.boot,
    seq: fixture.journal.at(-1)?.seq ?? 0,
    server_time_ms: now,
    status: structuredClone(fixture.status),
    peers: structuredClone(fixture.peers),
    traffic_tail: traffic,
    pending: structuredClone(fixture.pending),
    versions: structuredClone(fixture.status.versions),
    last_tick: {
      at_ms: now,
      traffic: traffic.map((series) => ({
        network: series.network,
        node: series.node,
        rx: series.samples.at(-1)?.rx_bytes ?? 0,
        tx: series.samples.at(-1)?.tx_bytes ?? 0,
      })),
      links: structuredClone(Object.values(fixture.links).flat()),
    },
    ...overrides,
  };
}

/** A tick `step` ms after `boot`'s, with every counter grown by `rx`/`tx` bytes. */
export function tickAfter(previous: Tick, step: number, rx: number, tx = 0): Tick {
  return {
    at_ms: previous.at_ms + step,
    traffic: previous.traffic.map((t) => ({ ...t, rx: t.rx + rx, tx: t.tx + tx })),
    links: previous.links.map((l) => ({ ...l })),
  };
}

type Answer = unknown | ApiError | (() => unknown);

export class FakeStream implements StreamControl {
  closed = false;
  reconnects = 0;
  constructor(
    readonly resume: () => string,
    readonly on: StreamHandlers,
  ) {}
  close(): void {
    this.closed = true;
  }
  reconnect(): void {
    this.reconnects += 1;
  }
}

/** A Store whose network, frames and visibility the test controls. */
export class Harness {
  readonly answers = new Map<string, Answer>();
  readonly calls: string[] = [];
  readonly streams: FakeStream[] = [];
  frames: (() => void)[] = [];
  visible = true;
  #visibility: ((visible: boolean) => void) | null = null;
  readonly store: Store;

  constructor() {
    const deps: Deps = {
      get: <T>(path: string): Promise<T> => {
        this.calls.push(path);
        const answer = this.answers.get(path.split('?')[0] ?? path);
        if (answer instanceof ApiError) return Promise.reject(answer);
        if (answer === undefined) return Promise.reject(new ApiError('internal', 500, path, null));
        return Promise.resolve((typeof answer === 'function' ? answer() : answer) as T);
      },
      connect: (resume, on) => {
        const stream = new FakeStream(resume, on);
        this.streams.push(stream);
        return stream;
      },
      frame: (flush) => {
        this.frames.push(flush);
      },
      visible: () => this.visible,
      watchVisibility: (changed) => {
        this.#visibility = changed;
      },
    };
    this.store = new Store(deps);
  }

  /** The stream the store has open now. */
  get stream(): FakeStream {
    const open = this.streams.filter((s) => !s.closed).at(-1);
    if (!open) throw new Error('no open stream');
    return open;
  }

  /** Runs the frames requested so far. */
  frame(): void {
    const frames = this.frames;
    this.frames = [];
    for (const run of frames) run();
  }

  setVisible(visible: boolean): void {
    this.visible = visible;
    this.#visibility?.(visible);
  }

  fail(path: string, code: ApiErrorCode, status: number): void {
    this.answers.set(path, new ApiError(code, status, code, null));
  }
}

/** One kind's fields of an entry. */
type Body = Entry extends infer E ? (E extends Entry ? Omit<E, 'seq' | 'at_ms'> : never) : never;

/** An entry at `at_ms`, given its kind's fields. */
export function entry(seq: number, at_ms: number, body: Body): Entry {
  return { seq, at_ms, ...body } as Entry;
}

/** Lets pending promise callbacks run under fake timers. */
export async function settle(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
}
