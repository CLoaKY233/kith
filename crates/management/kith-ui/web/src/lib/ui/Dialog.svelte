<!--
@component
Dialog (spec §5): Bits Dialog, max width 440, `--r-3`, `--shadow-modal`, plain scrim (no
backdrop-filter). Esc closes; focus returns to the element that opened it. No modal ever
opens from another modal.
Props:
  - open: boolean, bindable.
  - title: string (from copy.ts).
  - description?: string.
  - children: the body.
  - footer?: Snippet, the buttons row (right-aligned, 12 px apart).
  - onOpenChange?: (open: boolean) => void.
-->
<script lang="ts">
  import { Dialog } from 'bits-ui';
  import type { Snippet } from 'svelte';

  let {
    open = $bindable(false),
    title,
    description,
    children,
    footer,
    onOpenChange,
  }: {
    open?: boolean;
    title: string;
    description?: string;
    children?: Snippet;
    footer?: Snippet;
    onOpenChange?: (open: boolean) => void;
  } = $props();
</script>

<Dialog.Root bind:open {onOpenChange}>
  <Dialog.Portal>
    <Dialog.Overlay class="kith-scrim" />
    <Dialog.Content class="kith-dialog">
      <Dialog.Title class="t-heading">{title}</Dialog.Title>
      {#if description}
        <Dialog.Description class="kith-dialog-desc">{description}</Dialog.Description>
      {/if}
      {@render children?.()}
      {#if footer}<div class="kith-dialog-footer">{@render footer()}</div>{/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  :global(.kith-scrim) {
    position: fixed;
    inset: 0;
    z-index: var(--z-dialog);
    background: var(--scrim);
  }
  :global(.kith-dialog) {
    position: fixed;
    left: 50%;
    top: 50%;
    z-index: var(--z-dialog);
    width: min(var(--dialog), calc(100vw - 2 * var(--gutter-mobile)));
    transform: translate(-50%, -50%);
    display: grid;
    gap: var(--s-3);
    padding: var(--s-5);
    border-radius: var(--r-3);
    background: var(--surface);
    color: var(--ink-1);
    box-shadow: var(--shadow-modal);
  }
  :global(.kith-dialog[data-state='open']) {
    animation: kith-dialog-in var(--d-dialog) var(--e-pop);
  }
  :global(.kith-dialog-desc) {
    color: var(--ink-2);
  }
  :global(.kith-dialog-footer) {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-3);
    margin-top: var(--s-3);
  }
  @keyframes -global-kith-dialog-in {
    from {
      opacity: 0;
      transform: translate(-50%, -50%) scale(var(--dialog-scale-from));
    }
  }
</style>
