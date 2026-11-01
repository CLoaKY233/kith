// The screen-reader announcer (spec §11.1): one polite region, one message at a time, at
// least ANNOUNCER_GAP_MS apart, repeats coalesced. The assertive region is only for the
// daemon being lost and recovered. App.svelte renders both regions.

import { ANNOUNCER_GAP_MS } from './live/timing';

export type Politeness = 'polite' | 'assertive';

type Sink = (politeness: Politeness, message: string) => void;

let sink: Sink | null = null;
const queue: string[] = [];
let busy = false;

function drain(): void {
  if (busy || !sink) return;
  const next = queue.shift();
  if (next === undefined) return;
  busy = true;
  sink('polite', next);
  setTimeout(() => {
    busy = false;
    drain();
  }, ANNOUNCER_GAP_MS);
}

/** Connects the live regions; App.svelte calls this once. */
export function attachAnnouncer(target: Sink): void {
  sink = target;
  drain();
}

/** Queues a message. Assertive messages skip the queue. */
export function announce(message: string, politeness: Politeness = 'polite'): void {
  if (politeness === 'assertive') {
    sink?.('assertive', message);
    return;
  }
  if (queue.includes(message)) return;
  queue.push(message);
  drain();
}
