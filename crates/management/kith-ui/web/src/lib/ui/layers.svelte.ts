// Shared behaviour of the overlay components (spec §5, §10): where focus goes back to when a
// layer closes, the rule that no modal opens from another modal, and the stacking of
// popovers, menus and tooltips that open inside a modal.

import { getContext, setContext } from 'svelte';
import { MediaQuery } from 'svelte/reactivity';

/** Touch input: tooltips are skipped and targets grow to 44 px (§5, §11.4). */
export const coarse = new MediaQuery('(pointer: coarse)', false);
/** Below 40rem the page uses the mobile layout (§4.4). */
export const narrow = new MediaQuery('(width < 40rem)', false);

/** Gap between a floating layer and its anchor: --s-2. */
export const FLOAT_OFFSET = 8;
/** Gap between a menu and its trigger: --s-1. */
export const MENU_OFFSET = 4;

const MODAL = Symbol('kith-modal');

/** Marks the subtree as inside a modal, so floating layers stack above it. */
export function provideModal(): void {
  setContext(MODAL, true);
}

/** True when the calling component renders inside a Dialog, AlertDialog, Sheet or palette. */
export function insideModal(): boolean {
  return getContext<boolean | undefined>(MODAL) === true;
}

type Layer = { open: boolean; opener: HTMLElement | null };

// eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping, never rendered
const layers = new Map<string, Layer>();
let next = 0;

/**
 * The element focus should return to when a layer opened now closes. A menu item resolves to
 * its menu's trigger, and an element inside a closing modal resolves to that modal's own
 * opener, because both are gone by the time the new layer closes.
 */
export function opener(active: Element | null = document.activeElement): HTMLElement | null {
  if (!(active instanceof HTMLElement) || active === document.body) return null;
  const host = active.closest<HTMLElement>('[data-kith-layer]')?.dataset['kithLayer'];
  const layer = host === undefined ? undefined : layers.get(host);
  if (layer && !layer.open) return layer.opener;
  const menu = active.closest('[role="menu"]');
  if (menu?.id) {
    const trigger = document.querySelector(`[aria-controls="${CSS.escape(menu.id)}"]`);
    if (trigger instanceof HTMLElement) return trigger;
  }
  return active;
}

const TABBABLE =
  'input:not([type="hidden"]):not([disabled]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])';

/** The first control inside `root` that takes focus from the keyboard, if any. */
export function firstTabbable(root: HTMLElement | null): HTMLElement | null {
  if (!root) return null;
  for (const el of root.querySelectorAll<HTMLElement>(TABBABLE)) {
    if (!el.closest('[hidden], [inert], [aria-hidden="true"]') && el.getClientRects().length > 0) {
      return el;
    }
  }
  return null;
}

/** Moves focus back to `target` if it is still on the page. */
export function restoreFocus(target: HTMLElement | null): void {
  if (target?.isConnected) target.focus();
}

export type ModalLayer = {
  /** Put on the content element as `data-kith-layer`. */
  readonly id: string;
  /** Pass to the Bits content's `onCloseAutoFocus`. */
  readonly onCloseAutoFocus: (event: Event) => void;
  /** Runs `task` once the layer has closed and focus is back on its opener. */
  readonly afterClose: (task: () => void) => void;
};

/**
 * Registers a modal layer: records its opener when it opens and returns focus there when it
 * closes, even when the opener was a menu item or the open was driven by the URL.
 */
export function modalLayer(isOpen: () => boolean): ModalLayer {
  provideModal();
  const id = `layer-${(next += 1)}`;
  const layer: Layer = { open: false, opener: null };
  const pending: (() => void)[] = [];
  layers.set(id, layer);

  $effect.pre(() => {
    const open = isOpen();
    if (open === layer.open) return;
    if (open) {
      if (import.meta.env.DEV && [...layers.values()].some((other) => other.open)) {
        console.warn('kith: a modal opened from another modal (spec §5).');
      }
      layer.opener = opener();
    }
    layer.open = open;
  });

  $effect(() => () => {
    layers.delete(id);
  });

  return {
    id,
    onCloseAutoFocus(event: Event): void {
      event.preventDefault();
      const target = layer.opener;
      setTimeout(() => {
        restoreFocus(target);
        for (const task of pending.splice(0)) task();
      });
    },
    afterClose(task: () => void): void {
      pending.push(task);
    },
  };
}
