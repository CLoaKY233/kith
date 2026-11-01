// Screen state for Devices and Device detail that outlives one mount: the list's sort and
// scroll per network, the in-place row progress other flows write, and a pending rename.
// App.svelte mounts a new screen on every route change, so this cannot live in a component.

import { SvelteMap } from 'svelte/reactivity';

/** §6.3 `Sort: Name ▾`: by name, or by traffic frozen at a moment until `Re-sort`. */
export type DeviceSort =
  | { readonly by: 'name' }
  | { readonly by: 'traffic'; readonly order: readonly string[]; readonly at: number };

/** The sort per network id. */
export const deviceSort = new SvelteMap<string, DeviceSort>();

/**
 * In-place progress text per `net/node`, shown in the row instead of its path. A flow that
 * changes a device writes it, for example RemoveDevice with `Removing… 3 of 5 have dropped
 * phone`, and deletes it when done.
 */
export const rowProgress = new SvelteMap<string, string>();

/** The device last opened from the list per network id, refocused when its page closes. */
// eslint-disable-next-line svelte/prefer-svelte-reactivity -- read once on mount, never rendered
export const lastOpened = new Map<string, string>();

/** The list's scroll position per network id, restored when a full-page detail closes (§4.4). */
// eslint-disable-next-line svelte/prefer-svelte-reactivity -- read once on mount, never rendered
export const listScroll = new Map<string, number>();

/** A rename asked for from a row's ⋯ menu; the detail starts editing when it opens. */
export const renameRequest = $state<{ key: string | null }>({ key: null });

/** A member's ask from a row's ⋯ menu; the detail opens the Ask-an-admin sheet for it. */
export const askRequest = $state<{ key: string | null; request: string | null }>({
  key: null,
  request: null,
});

/** Whether the detail was opened from the list, so its back arrow can go back in history. */
export const openedFromList = $state<{ value: boolean }>({ value: false });

// eslint-disable-next-line svelte/prefer-svelte-reactivity -- a memo, never rendered
const seenBase = new Map<string, { secs: number | null; at: number | null }>();

/**
 * When a device was last heard from, in server ms, from the `last_seen_secs` it had when
 * first read. The store keeps that field as the snapshot gave it, so the age is anchored
 * once and then grows with the clock.
 */
export function snapshotSeenAt(key: string, secs: number | null, nowMs: number): number | null {
  const base = seenBase.get(key);
  if (base && base.secs === secs) return base.at;
  const at = secs === null ? null : nowMs - secs * 1000;
  seenBase.set(key, { secs, at });
  return at;
}
