/* eslint-disable svelte/prefer-svelte-reactivity -- the timers and held keys are
   bookkeeping that no markup reads; `changed` and `unseen` are the reactive surface. */

// The change-tint tracker (spec §8.4). A state change marks its element for TINT_MS; the
// element reads `changed.get(key)` and runs the `--changed` fade while it is set. The
// value is a stamp that grows on every mark, so a second change within the 4 s restarts
// the fade (key the animation on it). Changes made while the tab is hidden are held and
// tinted when it returns; `unseen` counts them for the `(n) ` title prefix.

import { SvelteMap } from 'svelte/reactivity';
import { TINT_MS } from './timing';

export class Tints {
  /** Tinted elements now, key → stamp. */
  readonly changed = new SvelteMap<string, number>();
  /** Elements that changed while the tab was hidden. */
  unseen = $state(0);

  #visible: boolean;
  #stamp = 0;
  #timers = new Map<string, ReturnType<typeof setTimeout>>();
  /** Held while hidden: key → whether it counts toward `unseen`. */
  #held = new Map<string, boolean>();

  constructor(visible = true) {
    this.#visible = visible;
  }

  /** Marks `key` as changed. `counts` is false for changes too small for the title. */
  mark(key: string, counts = true): void {
    if (this.#visible) {
      this.#show(key);
      return;
    }
    this.#held.set(key, counts || (this.#held.get(key) ?? false));
    this.unseen = [...this.#held.values()].filter(Boolean).length;
  }

  /** Drops any tint on `key`, for an element that left. */
  forget(key: string): void {
    const timer = this.#timers.get(key);
    if (timer !== undefined) clearTimeout(timer);
    this.#timers.delete(key);
    this.changed.delete(key);
    if (this.#held.delete(key)) this.unseen = [...this.#held.values()].filter(Boolean).length;
  }

  /** The tab became visible or hidden. On return, held changes are tinted now. */
  setVisible(visible: boolean): void {
    this.#visible = visible;
    if (!visible) return;
    for (const key of this.#held.keys()) this.#show(key);
    this.#held.clear();
    this.unseen = 0;
  }

  /** Clears every tint, for a store that starts over. */
  clear(): void {
    for (const timer of this.#timers.values()) clearTimeout(timer);
    this.#timers.clear();
    this.#held.clear();
    this.changed.clear();
    this.unseen = 0;
  }

  #show(key: string): void {
    this.#stamp += 1;
    this.changed.set(key, this.#stamp);
    const timer = this.#timers.get(key);
    if (timer !== undefined) clearTimeout(timer);
    this.#timers.set(
      key,
      setTimeout(() => {
        this.#timers.delete(key);
        this.changed.delete(key);
      }, TINT_MS),
    );
  }
}
