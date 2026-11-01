// The one EventSource (spec §2.6, §8.1, §8.5). It resumes from `boot:seq`, reconnects
// with backoff (0.5 s, 1 s, 2 s, then 4 s with jitter), retries at once when the tab
// becomes visible, and hands parsed events to the store. It holds no data of its own.

import type { Entry, Gap, Reset, Tick } from '../types.gen';
import { RECONNECT_BACKOFF_MS, RECONNECT_JITTER } from './timing';

export interface StreamHandlers {
  entry(entry: Entry): void;
  tick(tick: Tick): void;
  reset(reset: Reset): void;
  gap(gap: Gap): void;
  daemonDown(): void;
  daemonUp(): void;
  /** The connection opened, the first time or again. */
  open(): void;
  /** The connection failed or closed; the stream retries by itself. */
  error(): void;
}

/** What the store can do with an open stream. */
export interface StreamControl {
  /** Closes for good: no more events, no retries. */
  close(): void;
  /** Drops the connection and opens a new one now, from the current resume point. */
  reconnect(): void;
}

/** The parts of the browser the stream uses; tests pass fakes. */
export interface StreamEnv {
  readonly EventSource: new (url: string) => EventSource;
  readonly document: Pick<Document, 'visibilityState' | 'addEventListener' | 'removeEventListener'>;
  readonly random: () => number;
}

function browserEnv(): StreamEnv {
  return { EventSource, document, random: Math.random };
}

function parse<T>(event: Event): T | null {
  if (!(event instanceof MessageEvent) || typeof event.data !== 'string') return null;
  try {
    return JSON.parse(event.data) as T;
  } catch {
    return null;
  }
}

/** The wait before retry number `attempt` (0-based), in ms. */
export function backoff(attempt: number, random: () => number): number {
  const steps = RECONNECT_BACKOFF_MS;
  const last = steps.length - 1;
  const base = steps[Math.min(attempt, last)] ?? steps[last] ?? 0;
  if (attempt < last) return base;
  return base * (1 + (random() * 2 - 1) * RECONNECT_JITTER);
}

/** The stream's URL. EventSource cannot set `Last-Event-ID` itself on a fresh connect,
 * so the resume point goes in the query; the server reads either. */
export function streamUrl(resume: string): string {
  return `/api/events?last=${encodeURIComponent(resume)}`;
}

/** Opens `/api/events` from `resume()` (`boot:seq`), read again on every reconnect. */
export function connect(
  resume: () => string,
  on: StreamHandlers,
  env: StreamEnv = browserEnv(),
): StreamControl {
  let source: EventSource | null = null;
  let attempt = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let closed = false;

  const drop = (): void => {
    if (!source) return;
    const old = source;
    source = null;
    old.close();
  };

  const cancelRetry = (): void => {
    if (timer === null) return;
    clearTimeout(timer);
    timer = null;
  };

  const open = (): void => {
    if (closed) return;
    cancelRetry();
    drop();
    const current = new env.EventSource(streamUrl(resume()));
    source = current;
    // Events from a source this stream already dropped are ignored.
    const live = (): boolean => source === current;
    const listen = <T>(name: string, handle: (value: T) => void): void => {
      current.addEventListener(name, (event) => {
        if (!live()) return;
        const value = parse<T>(event);
        if (value !== null) handle(value);
      });
    };
    current.addEventListener('open', () => {
      if (!live()) return;
      attempt = 0;
      on.open();
    });
    listen<Entry>('entry', (entry) => on.entry(entry));
    listen<Tick>('tick', (tick) => on.tick(tick));
    listen<Reset>('reset', (reset) => on.reset(reset));
    listen<Gap>('gap', (gap) => on.gap(gap));
    current.addEventListener('daemon_down', () => {
      if (live()) on.daemonDown();
    });
    current.addEventListener('daemon_up', () => {
      if (live()) on.daemonUp();
    });
    current.addEventListener('error', () => {
      if (!live()) return;
      // The browser would retry on its own with its own timing; this stream owns retries.
      drop();
      on.error();
      schedule();
    });
  };

  const schedule = (): void => {
    if (closed || timer !== null) return;
    const delay = backoff(attempt, env.random);
    attempt += 1;
    timer = setTimeout(() => {
      timer = null;
      open();
    }, delay);
  };

  const onVisibility = (): void => {
    if (env.document.visibilityState === 'visible' && source === null && !closed) open();
  };

  env.document.addEventListener('visibilitychange', onVisibility);
  open();

  return {
    close: () => {
      closed = true;
      cancelRetry();
      drop();
      env.document.removeEventListener('visibilitychange', onVisibility);
    },
    reconnect: () => {
      if (closed) return;
      attempt = 0;
      open();
    },
  };
}
