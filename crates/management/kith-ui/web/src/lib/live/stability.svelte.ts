/* eslint-disable svelte/prefer-svelte-reactivity -- the plain Maps and Sets here are
   bookkeeping and local values that no markup reads; `shown` is the reactive surface. */

// Stability rules (spec §5 PathBadge, §8.7). Live data must not make the page jump:
// - a path badge changes style only after the path held for PATH_HYSTERESIS_MS, and
//   reads `flapping` after more than FLAP_COUNT changes in FLAP_WINDOW_MS;
// - lists never reorder while hovered or focused, nor for SORT_FREEZE_MS after the
//   pointer leaves (`Hold`, `stableOrder`);
// - a row inserted above the viewport keeps the scroll anchored (`keepScrollAnchor`);
// - Activity pauses once scrolled more than JOURNAL_PAUSE_PX from the top
//   (`JournalCursor`), and a pill counts what arrived since.

import { SvelteMap } from 'svelte/reactivity';
import type { PathKind } from '../types.gen';
import { FLAP_COUNT, FLAP_WINDOW_MS, PATH_HYSTERESIS_MS, SORT_FREEZE_MS } from './timing';

/** The path a badge shows. */
export type ShownPath = { readonly path: PathKind | null; readonly flapping: boolean };

type Timer = ReturnType<typeof setTimeout>;

/**
 * Paths as badges show them. The raw path comes from journal entries; the shown one
 * follows after it held for PATH_HYSTERESIS_MS, measured from the entry's own time, so
 * replayed history settles at once. Going online or offline shows at once: only a change
 * of style (direct, relayed, stealth) waits.
 */
export class Paths {
  readonly shown = new SvelteMap<string, ShownPath>();

  #raw = new Map<string, { path: PathKind | null; sinceMs: number }>();
  #flaps = new Map<string, number[]>();
  #settle = new Map<string, Timer>();
  #unflap = new Map<string, Timer>();
  readonly #now: () => number;
  readonly #onStyleChange: (key: string) => void;

  /**
   * @param now server time now, in ms.
   * @param onStyleChange called when a shown path changes from one kind to another.
   */
  constructor(now: () => number, onStyleChange: (key: string) => void) {
    this.#now = now;
    this.#onStyleChange = onStyleChange;
  }

  /** Sets the path with no wait and no tint, for a first snapshot. */
  seed(key: string, path: PathKind | null, atMs: number): void {
    this.#raw.set(key, { path, sinceMs: atMs });
    this.#cancel(this.#settle, key);
    this.shown.set(key, { path, flapping: this.#flapping(key) });
  }

  /**
   * Records the path now. `flaps` holds the times of style changes since the last call;
   * `atMs` is when the path became what it is.
   */
  observe(key: string, path: PathKind | null, atMs: number, flaps: readonly number[] = []): void {
    if (flaps.length > 0) this.#flaps.set(key, [...(this.#flaps.get(key) ?? []), ...flaps]);
    const raw = this.#raw.get(key);
    if (!raw || raw.path !== path) this.#raw.set(key, { path, sinceMs: atMs });
    const shown = this.shown.get(key);
    if (!shown || shown.path === null || path === null) {
      this.#apply(key);
    } else if (shown.path === path) {
      this.#cancel(this.#settle, key);
    } else {
      this.#schedule(key);
    }
    this.#refreshFlapping(key);
  }

  /** Forgets a device that left. */
  forget(key: string): void {
    this.#cancel(this.#settle, key);
    this.#cancel(this.#unflap, key);
    this.#raw.delete(key);
    this.#flaps.delete(key);
    this.shown.delete(key);
  }

  clear(): void {
    for (const key of [...this.#raw.keys()]) this.forget(key);
  }

  #schedule(key: string): void {
    const raw = this.#raw.get(key);
    if (!raw) return;
    this.#cancel(this.#settle, key);
    const wait = PATH_HYSTERESIS_MS - (this.#now() - raw.sinceMs);
    if (wait <= 0) {
      this.#apply(key);
      return;
    }
    this.#settle.set(
      key,
      setTimeout(() => {
        this.#settle.delete(key);
        this.#apply(key);
      }, wait),
    );
  }

  #apply(key: string): void {
    this.#cancel(this.#settle, key);
    const raw = this.#raw.get(key);
    if (!raw) return;
    const before = this.shown.get(key);
    this.shown.set(key, { path: raw.path, flapping: this.#flapping(key) });
    if (before && before.path !== null && raw.path !== null && before.path !== raw.path) {
      this.#onStyleChange(key);
    }
  }

  #flapping(key: string): boolean {
    const since = this.#now() - FLAP_WINDOW_MS;
    const recent = (this.#flaps.get(key) ?? []).filter((at) => at > since);
    if (recent.length > 0) this.#flaps.set(key, recent);
    else this.#flaps.delete(key);
    return recent.length > FLAP_COUNT;
  }

  /** Updates `flapping` now, and again when the oldest change leaves the window. */
  #refreshFlapping(key: string): void {
    this.#cancel(this.#unflap, key);
    const flapping = this.#flapping(key);
    const shown = this.shown.get(key);
    if (shown && shown.flapping !== flapping) this.shown.set(key, { ...shown, flapping });
    const oldest = this.#flaps.get(key)?.[0];
    if (oldest === undefined) return;
    this.#unflap.set(
      key,
      setTimeout(
        () => {
          this.#unflap.delete(key);
          this.#refreshFlapping(key);
        },
        Math.max(0, oldest + FLAP_WINDOW_MS - this.#now()),
      ),
    );
  }

  #cancel(timers: Map<string, Timer>, key: string): void {
    const timer = timers.get(key);
    if (timer !== undefined) clearTimeout(timer);
    timers.delete(key);
  }
}

/**
 * Whether a list is held still: the pointer is over it, focus is inside it, or the
 * pointer left less than SORT_FREEZE_MS ago. Attach it to the list element with
 * `{@attach hold.attach}` and pass `hold.held` to `stableOrder`.
 */
export class Hold {
  #pointer = $state(false);
  #focus = $state(false);
  #lingering = $state(false);
  #timer: Timer | null = null;

  get held(): boolean {
    return this.#pointer || this.#focus || this.#lingering;
  }

  pointerEnter(): void {
    this.#stopLinger();
    this.#pointer = true;
  }

  pointerLeave(): void {
    if (!this.#pointer) return;
    this.#pointer = false;
    this.#stopLinger();
    this.#lingering = true;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.#lingering = false;
    }, SORT_FREEZE_MS);
  }

  focusIn(): void {
    this.#focus = true;
  }

  focusOut(): void {
    this.#focus = false;
  }

  /** Wires the list element's pointer and focus events. */
  readonly attach = (node: HTMLElement): (() => void) => {
    const enter = (): void => this.pointerEnter();
    const leave = (): void => this.pointerLeave();
    const focusIn = (): void => this.focusIn();
    const focusOut = (event: FocusEvent): void => {
      // Focus moving between rows stays inside the list.
      if (event.relatedTarget instanceof Node && node.contains(event.relatedTarget)) return;
      this.focusOut();
    };
    node.addEventListener('pointerenter', enter);
    node.addEventListener('pointerleave', leave);
    node.addEventListener('focusin', focusIn);
    node.addEventListener('focusout', focusOut);
    return () => {
      node.removeEventListener('pointerenter', enter);
      node.removeEventListener('pointerleave', leave);
      node.removeEventListener('focusin', focusIn);
      node.removeEventListener('focusout', focusOut);
      this.#stopLinger();
    };
  };

  #stopLinger(): void {
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    this.#lingering = false;
  }
}

/**
 * The order to show. Free, it is `next`. Held, rows already shown keep their places,
 * rows that left are dropped, and new rows go at the end, so nothing under the pointer
 * or focus moves; the sorted order returns once the hold ends.
 */
export function stableOrder<K>(shown: readonly K[], next: readonly K[], held: boolean): K[] {
  if (!held) return [...next];
  const present = new Set(next);
  const kept = shown.filter((key) => present.has(key));
  const keptSet = new Set(kept);
  return [...kept, ...next.filter((key) => !keptSet.has(key))];
}

/** How far Activity scrolls from the top before it stops taking new rows (§8.7). */
export const JOURNAL_PAUSE_PX = 48;

/**
 * Pauses the Activity feed while the reader is scrolled into it. Positions count every
 * entry ever appended (`live.journalTotal`), so the cap dropping old rows does not
 * shift them. Call `scrolled` from the feed's scroll handler; `split` gives the rows to
 * show and the newer ones the `12 new ↑` pill counts; `resume` (pill click or Home)
 * shows everything again.
 */
export class JournalCursor {
  paused = $state(false);
  /** `journalTotal` when the feed paused. */
  #at = $state(0);

  scrolled(fromTopPx: number, total: number): void {
    if (fromTopPx > JOURNAL_PAUSE_PX) {
      if (!this.paused) {
        this.paused = true;
        this.#at = total;
      }
    } else if (this.paused) {
      this.resume();
    }
  }

  resume(): void {
    this.paused = false;
  }

  /** `journal` is oldest first, as the store holds it; `total` is `live.journalTotal`. */
  split<T>(journal: readonly T[], total: number): { shown: readonly T[]; newer: readonly T[] } {
    if (!this.paused) return { shown: journal, newer: [] };
    const count = Math.min(journal.length, Math.max(0, total - this.#at));
    const cut = journal.length - count;
    return { shown: journal.slice(0, cut), newer: journal.slice(cut) };
  }
}

/** The nearest ancestor that scrolls, or the page. */
function scrollerOf(node: HTMLElement): HTMLElement {
  for (let el = node.parentElement; el; el = el.parentElement) {
    const { overflowY } = getComputedStyle(el);
    if ((overflowY === 'auto' || overflowY === 'scroll') && el.scrollHeight > el.clientHeight) return el;
  }
  return document.scrollingElement instanceof HTMLElement ? document.scrollingElement : document.body;
}

/**
 * Keeps the first row in view where it is when rows are inserted or removed above it.
 * Rows must be the list's direct children. Browsers with CSS scroll anchoring already
 * do this, so there it does nothing; WebKit needs it. At the very top nothing is
 * anchored, so a new first row shows.
 */
export function keepScrollAnchor(list: HTMLElement): () => void {
  if (typeof CSS !== 'undefined' && CSS.supports('overflow-anchor', 'auto')) return () => undefined;
  const scroller = scrollerOf(list);
  const page = scroller === document.scrollingElement || scroller === document.body;
  const viewTop = (): number => (page ? 0 : scroller.getBoundingClientRect().top);
  let anchor: Element | null = null;
  let anchorTop = 0;

  const pick = (): void => {
    anchor = null;
    if (scroller.scrollTop <= 0) return;
    const top = viewTop();
    for (const row of list.children) {
      const rect = row.getBoundingClientRect();
      if (rect.top >= top) {
        anchor = row;
        anchorTop = rect.top;
        return;
      }
    }
  };

  const observer = new MutationObserver(() => {
    if (anchor?.isConnected) {
      const moved = anchor.getBoundingClientRect().top - anchorTop;
      if (moved !== 0) scroller.scrollTop += moved;
    }
    pick();
  });
  const target: EventTarget = page ? window : scroller;
  target.addEventListener('scroll', pick, { passive: true });
  observer.observe(list, { childList: true });
  pick();
  return () => {
    observer.disconnect();
    target.removeEventListener('scroll', pick);
  };
}
