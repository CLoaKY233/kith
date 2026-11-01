// The single store (spec §8.2). Public surface, frozen for the other workstreams:
//
//   live.*         readonly reactive state: read it in markup or $derived, never assign it.
//   actions.*      every change the page can make; each returns an ActionHandle.
//   start()        hydrates from the boot snapshot and opens the stream; main.ts calls it.
//   onEntry()      calls back with each new journal entry, after the frame that applied it.
//   notifications  the browser-notification opt-in (§7.12) for Settings and Activity.
//
// The state and its live wiring are in core.svelte.ts; actions run through runner.ts, and
// watch.svelte.ts follows the journal and connection to mark actions seen, notify and
// announce. This file makes the page's one instance with the browser's fetch, EventSource
// and frames.

import { go, href, parse } from '../../router';
import { api } from '../api';
import type { Boot, Entry } from '../types.gen';
import { createActions, type Actions } from './actions';
import { readInlineBoot } from './boot';
import { Store, viewOf, type Deps, type Live } from './core.svelte';
import { Notifications } from './notifications.svelte';
import { browserEnv } from './notify';
import { Runner } from './runner';
import { connect } from './stream';
import { watch } from './watch.svelte';

const browser: Deps = {
  get: (path) => api.get(path),
  connect: (resume, on) => connect(resume, on),
  frame: (flush) => requestAnimationFrame(() => flush()),
  visible: () => typeof document === 'undefined' || document.visibilityState !== 'hidden',
  watchVisibility: (changed) => {
    if (typeof document === 'undefined') return;
    document.addEventListener('visibilitychange', () =>
      changed(document.visibilityState !== 'hidden'),
    );
  },
};

const store = new Store(browser);

export type { Live, Actions };
export type { JournalGap } from './core.svelte';
export type { ShownPath } from './stability.svelte';

/** The store, read-only. */
export const live: Live = viewOf(store);

const runner = new Runner(store);

/** Every change and on-demand read. Each returns an ActionHandle (§16 C, runner.ts). */
export const actions: Actions = createActions(store, runner);

/** Feeds a propagation ack (§2.12 `op_seen`) to the action that signed `op`. */
export function ackOp(op: string, by: number, of: number): void {
  runner.ack(op, by, of);
}

store.onAck((op, by, of) => ackOp(op, by, of));

/** Calls `listener` with each new journal entry; returns a function that stops it. */
export function onEntry(listener: (entry: Entry) => void): () => void {
  return store.onEntry(listener);
}

/** Browser notification opt-in and delivery (§7.12): Settings and the Activity card. */
export const notifications = new Notifications(
  browserEnv(live.now, (target) => {
    if (!target) return;
    const network = target.network ?? currentNetwork();
    if (network === null) return;
    if ('id' in target) {
      go(`${href({ name: 'overview', net: network })}?need=${encodeURIComponent(target.id)}`);
    } else if (target.page === 'device') {
      go(href({ name: 'device', net: network, dev: target.device }));
    } else {
      go(href({ name: target.page, net: network }));
    }
  }),
);

/** The network the page shows now, else this device's first. */
function currentNetwork(): string | null {
  const route = typeof location === 'undefined' ? null : parse(location.pathname);
  if (route && 'net' in route) return route.net;
  return store.status?.networks[0]?.id ?? null;
}

watch(live, runner, notifications, onEntry);

/** Hydrates from `#boot` (or /api/boot) and opens the stream. Safe to call more than once.
 * With an inline snapshot the state is complete before this returns, so the first frame
 * has it (§8.1). */
export function start(): Promise<void> {
  return store.start(readInlineBoot(typeof document === 'undefined' ? null : document));
}

/** Test hook: loads a snapshot without a server. */
export function hydrateForTest(snapshot: Boot): void {
  store.hydrate(snapshot);
}
