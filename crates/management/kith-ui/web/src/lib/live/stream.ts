// The one EventSource (spec §2.6, §8.1). It resumes from `boot:seq`, reconnects with
// backoff, and hands parsed events to the store; it holds no state of its own.

import type { Entry, Gap, Reset, Tick } from '../types.gen';
import { RECONNECT_BACKOFF_MS, RECONNECT_JITTER } from './timing';

export interface StreamHandlers {
  entry(entry: Entry): void;
  tick(tick: Tick): void;
  reset(reset: Reset): void;
  gap(gap: Gap): void;
  daemonDown(): void;
  daemonUp(): void;
  /** The connection opened (first time or again). */
  open(): void;
  /** The connection failed; the stream retries by itself. */
  error(): void;
}

function parse<T>(event: MessageEvent<string>): T | null {
  try {
    return JSON.parse(event.data) as T;
  } catch {
    return null;
  }
}

/** Opens `/api/events` from `resume` (`boot:seq`). Returns a function that closes it. */
export function connect(resume: () => string, on: StreamHandlers): () => void {
  let source: EventSource | null = null;
  let attempt = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let closed = false;

  const open = (): void => {
    if (closed) return;
    source?.close();
    // The first connect cannot set Last-Event-ID, so the resume point goes in the query.
    source = new EventSource(`/api/events?last=${encodeURIComponent(resume())}`);
    source.addEventListener('open', () => {
      attempt = 0;
      on.open();
    });
    source.addEventListener('entry', (event) => {
      const entry = parse<Entry>(event);
      if (entry) on.entry(entry);
    });
    source.addEventListener('tick', (event) => {
      const tick = parse<Tick>(event);
      if (tick) on.tick(tick);
    });
    source.addEventListener('reset', (event) => {
      const reset = parse<Reset>(event);
      if (reset) on.reset(reset);
    });
    source.addEventListener('gap', (event) => {
      const gap = parse<Gap>(event);
      if (gap) on.gap(gap);
    });
    source.addEventListener('daemon_down', () => on.daemonDown());
    source.addEventListener('daemon_up', () => on.daemonUp());
    source.addEventListener('error', () => {
      source?.close();
      source = null;
      on.error();
      schedule();
    });
  };

  const schedule = (): void => {
    if (closed || timer !== null) return;
    const base =
      RECONNECT_BACKOFF_MS[Math.min(attempt, RECONNECT_BACKOFF_MS.length - 1)] ?? 0;
    const last = attempt >= RECONNECT_BACKOFF_MS.length - 1;
    const delay = last ? base * (1 + (Math.random() * 2 - 1) * RECONNECT_JITTER) : base;
    attempt += 1;
    timer = setTimeout(() => {
      timer = null;
      open();
    }, delay);
  };

  const onVisible = (): void => {
    if (document.visibilityState === 'visible' && source === null) {
      if (timer !== null) clearTimeout(timer);
      timer = null;
      open();
    }
  };

  document.addEventListener('visibilitychange', onVisible);
  open();
  return () => {
    closed = true;
    if (timer !== null) clearTimeout(timer);
    source?.close();
    document.removeEventListener('visibilitychange', onVisible);
  };
}
