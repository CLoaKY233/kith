// Removals in flight, shared by RemoveDevice and the rows that show their progress, so
// a device list does not load the sheet.

import { SvelteMap } from 'svelte/reactivity';
import { copy as text } from '../lib/copy';
import { live as store } from '../lib/live/store.svelte';
import { REMOVED_HOLD_MS, ROW_ANIMATION_MS } from '../lib/live/timing';
import type { ActionHandle } from '../lib/types';
import type { Outcome } from '../lib/types.gen';

export type Removal = {
  readonly net: string;
  readonly node: string;
  readonly name: string;
  readonly handle: ActionHandle<Outcome>;
  /** Devices that were offline when it was signed: they drop it when they're back. */
  readonly offline: number;
};

const removals = new SvelteMap<string, Removal>();
let watching = false;

/** Forgets each removal once its device has left the list and its row has collapsed. */
function watch(): void {
  if (watching) return;
  watching = true;
  $effect.root(() => {
    $effect(() => {
      for (const [key, removal] of removals) {
        if (store.peers[removal.net]?.[removal.node]) continue;
        setTimeout(() => {
          if (removals.get(key) === removal) removals.delete(key);
        }, REMOVED_HOLD_MS + ROW_ANIMATION_MS);
      }
    });
  });
}

export function track(removal: Removal): void {
  const key = `${removal.net}/${removal.node}`;
  removals.set(key, removal);
  watch();
  void removal.handle.done.then((handle) => {
    if (handle.state !== 'accepted' && removals.get(key) === removal) removals.delete(key);
  });
}

export function removalOf(net: string, node: string): Removal | null {
  return removals.get(`${net}/${node}`) ?? null;
}

export function removalLine(removal: Removal): string {
  const seen = removal.handle.seen;
  if (!seen) return text.device.removing;
  if (seen.by < seen.of) return text.remove.progress(seen.by, seen.of, removal.name);
  return removal.offline > 0 ? text.remove.offlineLater(removal.offline) : text.device.removed;
}
