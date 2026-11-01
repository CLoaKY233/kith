// The screen-reader announcer (spec §11.1): one polite region, one message at a time, at
// least ANNOUNCER_GAP_MS apart, repeats coalesced. The assertive region is only for the
// daemon being lost and recovered. App.svelte renders both regions and attaches them.
//
// Announce only: join requests, blocked groups (at most once per triple per 5 min, via
// `key` and `every`), results of the person's own actions, and this device's connectivity.

import { ANNOUNCER_GAP_MS } from './live/timing';

export type Politeness = 'polite' | 'assertive';

export type AnnounceOptions = {
  readonly politeness?: Politeness;
  /** Groups messages about one subject, such as a blocked triple. */
  readonly key?: string;
  /** With `key`: drop the message if the same key was announced this recently, in ms. */
  readonly every?: number;
};

type Sink = (politeness: Politeness, message: string) => void;

// Old messages are worse than none: past this many waiting, the oldest is dropped.
const QUEUE_MAX = 3;
// A live region only speaks when its text changes, so a repeat gets this suffix toggled.
const REPEAT_MARK = ' ';

let sink: Sink | null = null;
let queue: string[] = [];
let busy: ReturnType<typeof setTimeout> | null = null;
let speaking: string | null = null;
const lastText: Record<Politeness, string> = { polite: '', assertive: '' };
const lastByKey = new Map<string, number>();

function emit(politeness: Politeness, message: string): void {
  const text = lastText[politeness] === message ? `${message}${REPEAT_MARK}` : message;
  lastText[politeness] = text;
  sink?.(politeness, text);
}

function drain(): void {
  if (busy !== null || !sink) return;
  const next = queue.shift();
  if (next === undefined) {
    speaking = null;
    return;
  }
  speaking = next;
  emit('polite', next);
  busy = setTimeout(() => {
    busy = null;
    drain();
  }, ANNOUNCER_GAP_MS);
}

/** Connects the live regions; App.svelte calls this once. Queued messages then play. */
export function attachAnnouncer(target: Sink): void {
  sink = target;
  drain();
}

/**
 * Queues a message for screen readers. Assertive messages skip the queue. A message equal
 * to one waiting or being spoken is dropped.
 */
export function announce(message: string, options: Politeness | AnnounceOptions = {}): void {
  const opts: AnnounceOptions = typeof options === 'string' ? { politeness: options } : options;
  if (message === '') return;
  if (opts.key !== undefined && opts.every !== undefined) {
    const now = Date.now();
    const last = lastByKey.get(opts.key);
    if (last !== undefined && now - last < opts.every) return;
    lastByKey.set(opts.key, now);
  }
  if (opts.politeness === 'assertive') {
    emit('assertive', message);
    return;
  }
  if (message === speaking || queue.includes(message)) return;
  queue.push(message);
  if (queue.length > QUEUE_MAX) queue = queue.slice(-QUEUE_MAX);
  drain();
}

/** Test hook: forgets the sink, the queue and every key. */
export function resetAnnouncer(): void {
  if (busy !== null) clearTimeout(busy);
  busy = null;
  sink = null;
  queue = [];
  speaking = null;
  lastText.polite = '';
  lastText.assertive = '';
  lastByKey.clear();
}
