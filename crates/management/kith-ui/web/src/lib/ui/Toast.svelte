<!--
@component
Toast host (spec §5): svelte-sonner, bottom-right on desktop and top below 40rem, at most 3
visible, 5 s, paused while hovered or focused and while the tab is hidden. Toasts are only for
the user's own actions; network events never toast. A toast with Undo stays 8 s (pass
`undoMs` for the allow-rule's 10 s). Error toasts carry a `circle-x` mark and stay until
dismissed. Every toast is also announced through announce(): sonner's own live regions are
switched off, so the page keeps one polite region (§11.1). Toasts are fixed, so the layout
never moves when one appears. Render once in App.svelte.
API (module exports):
  - toast.done(message, { undo?, undoMs? }): string | number. The result of the user's own
    action, with an optional Undo. Returns the toast's id.
  - toast.failed(message): string | number. A failure of the user's own action.
  - toast.dismiss(id?): removes one toast, or all of them.
Props: none.
-->
<script lang="ts" module>
  import { toast as sonner } from 'svelte-sonner';
  import { announce } from '../announce';
  import { copy } from '../copy';
  import { TOAST_MS, UNDO_MS } from '../live/timing';

  export type ToastOptions = { undo?: () => void; undoMs?: number };

  export const toast = {
    /** The result of the user's own action, with an optional Undo. */
    done(message: string, options: ToastOptions = {}): string | number {
      const { undo, undoMs } = options;
      announce(message);
      return sonner(message, {
        duration: undo ? (undoMs ?? UNDO_MS) : TOAST_MS,
        action: undo ? { label: copy.action.undo, onClick: () => undo() } : undefined,
      });
    },
    /** A failure of the user's own action: stays until dismissed. */
    failed(message: string): string | number {
      announce(message);
      return sonner.error(message, {
        duration: Number.POSITIVE_INFINITY,
        cancel: { label: copy.action.dismiss, onClick: () => undefined },
      });
    },
    /** Removes one toast, or every toast when no id is given. */
    dismiss(id?: string | number): void {
      sonner.dismiss(id);
    },
  };
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { Toaster } from 'svelte-sonner';
  import Icon from '../icons/Icon.svelte';
  import { TOAST_MAX } from '../live/timing';
  import { narrow } from './layers.svelte';

  let host: HTMLDivElement | null = $state(null);

  onMount(() => {
    const root = host;
    if (!root) return;
    const quiet = (): void => {
      for (const el of root.querySelectorAll('[aria-live]')) {
        if (el.getAttribute('aria-live') !== 'off') el.setAttribute('aria-live', 'off');
      }
      const region = root.querySelector('section');
      if (region && region.getAttribute('aria-label') !== copy.layer.toasts) {
        region.setAttribute('aria-label', copy.layer.toasts);
      }
    };
    quiet();
    const observer = new MutationObserver(quiet);
    observer.observe(root, {
      subtree: true,
      childList: true,
      attributeFilter: ['aria-live', 'aria-label'],
    });
    return () => observer.disconnect();
  });
</script>

<div bind:this={host} class="contents">
  <Toaster
    class="kith-toaster"
    position={narrow.current ? 'top-center' : 'bottom-right'}
    visibleToasts={TOAST_MAX}
    duration={TOAST_MS}
    gap={8}
    offset="var(--s-5)"
    mobileOffset={{
      top: 'calc(var(--gutter-mobile) + env(safe-area-inset-top))',
      right: 'var(--gutter-mobile)',
      bottom: 'var(--gutter-mobile)',
      left: 'var(--gutter-mobile)',
    }}
    pauseWhenPageIsHidden
    containerAriaLabel={copy.layer.toasts}
    closeButtonAriaLabel={copy.action.close}
    toastOptions={{
      unstyled: true,
      classes: {
        toast: 'kith-toast',
        content: 'kith-toast-content',
        icon: 'kith-toast-icon',
        actionButton: 'kith-toast-button',
        cancelButton: 'kith-toast-button',
      },
    }}
  >
    {#snippet errorIcon()}<Icon name="circle-x" />{/snippet}
  </Toaster>
</div>

<style>
  .contents {
    display: contents;
  }
  :global(.kith-toaster[data-sonner-toaster]) {
    z-index: var(--z-toast);
    font: var(--t-body);
    letter-spacing: var(--t-body-tracking);
  }
  :global(.kith-toast[data-sonner-toast]) {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    width: var(--width);
    min-height: var(--row);
    padding: var(--s-2) var(--s-2) var(--s-2) var(--s-4);
    border-radius: var(--r-3);
    border: var(--hairline) solid var(--line);
    background: var(--surface);
    color: var(--ink-1);
    box-shadow: var(--shadow-pop);
    transition-duration: var(--d-sheet);
    transition-timing-function: var(--e-sheet);
  }
  :global(.kith-toast[data-sonner-toast]:focus-visible) {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  :global(.kith-toast-content) {
    flex: 1;
    min-width: 0;
    padding-block: var(--s-1);
  }
  :global(.kith-toast-icon) {
    display: inline-grid;
    place-items: center;
    color: var(--danger);
  }
  :global(.kith-toast-button) {
    flex: none;
    height: var(--control);
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    font-weight: 600;
    color: var(--ink-1);
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  :global(.kith-toast-button:hover) {
    background: var(--sunken);
  }
  :global(.kith-toast-button:focus-visible) {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  @media (width < 40rem) {
    :global(.kith-toaster[data-sonner-toaster]) {
      left: var(--gutter-mobile);
      right: var(--gutter-mobile);
      width: auto;
      transform: none;
    }
    :global(.kith-toaster[data-sonner-toaster] .kith-toast[data-sonner-toast]) {
      left: 0;
      right: 0;
      width: 100%;
    }
  }
  @media (pointer: coarse) {
    :global(.kith-toast-button) {
      min-width: var(--target-touch);
      height: var(--target-touch);
    }
  }
  @media (forced-colors: active) {
    :global(.kith-toast[data-sonner-toast]) {
      border-color: CanvasText;
    }
  }
</style>
