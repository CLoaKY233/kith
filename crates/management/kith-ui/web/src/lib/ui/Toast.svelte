<!--
@component
Toast host (spec §5): svelte-sonner, bottom-right on desktop and top on mobile, at most 3,
5 s, paused on hover. Only for the user's own actions; network events never toast.
Error toasts stay until dismissed. Render once in App.svelte; raise toasts with
`toast.done(message, { undo })` and `toast.failed(message)` from this module.
Props: none.
-->
<script lang="ts" module>
  import { toast as sonner } from 'svelte-sonner';
  import { copy } from '../copy';
  import { TOAST_MS } from '../live/timing';

  export type ToastOptions = { undo?: () => void; undoMs?: number };

  export const toast = {
    /** The result of the user's own action, with an optional Undo. */
    done(message: string, options: ToastOptions = {}): void {
      const { undo, undoMs } = options;
      sonner(message, {
        duration: undoMs ?? TOAST_MS,
        action: undo ? { label: copy.action.undo, onClick: undo } : undefined,
      });
    },
    /** A failure of the user's own action: stays until dismissed. */
    failed(message: string): void {
      sonner.error(message, {
        duration: Number.POSITIVE_INFINITY,
        cancel: { label: copy.action.dismiss, onClick: () => undefined },
      });
    },
  };
</script>

<script lang="ts">
  import { Toaster } from 'svelte-sonner';
  import { TOAST_MAX } from '../live/timing';

  const narrow =
    typeof window !== 'undefined' && window.matchMedia('(max-width: 40rem)').matches;
</script>

<Toaster
  position={narrow ? 'top-center' : 'bottom-right'}
  visibleToasts={TOAST_MAX}
  duration={TOAST_MS}
  toastOptions={{ unstyled: true, class: 'kith-toast' }}
/>

<style>
  :global(.kith-toast) {
    z-index: var(--z-toast);
    display: flex;
    align-items: center;
    gap: var(--s-3);
    width: var(--recent-column);
    max-width: calc(100vw - 2 * var(--gutter-mobile));
    padding: var(--s-3) var(--s-4);
    border-radius: var(--r-3);
    border: var(--hairline) solid var(--line);
    background: var(--surface);
    color: var(--ink-1);
    box-shadow: var(--shadow-pop);
  }
  :global(.kith-toast [data-button]) {
    margin-left: auto;
    font-weight: 600;
    color: var(--accent);
    cursor: pointer;
  }
</style>
