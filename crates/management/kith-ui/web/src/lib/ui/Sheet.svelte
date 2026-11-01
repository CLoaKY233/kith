<!--
@component
Sheet (spec §5): a 480 px right-side panel for flows on desktop, full screen on mobile,
with the primary action pinned at the bottom. A sheet that is part of a flow stays open
until the outcome or the user cancels: outside clicks do not close it, Esc does.
Props:
  - open: boolean, bindable.
  - title: string (from copy.ts).
  - children: the body (scrolls).
  - footer?: Snippet, pinned at the bottom.
  - closeLabel: string, the close button's accessible name.
  - onOpenChange?: (open: boolean) => void. Screens map this to the `?add=` URL param.
-->
<script lang="ts">
  import { Dialog } from 'bits-ui';
  import type { Snippet } from 'svelte';
  import IconButton from './IconButton.svelte';

  let {
    open = $bindable(false),
    title,
    children,
    footer,
    closeLabel,
    onOpenChange,
  }: {
    open?: boolean;
    title: string;
    children?: Snippet;
    footer?: Snippet;
    closeLabel: string;
    onOpenChange?: (open: boolean) => void;
  } = $props();
</script>

<Dialog.Root bind:open {onOpenChange}>
  <Dialog.Portal>
    <Dialog.Overlay class="kith-scrim" />
    <Dialog.Content class="kith-sheet" interactOutsideBehavior="ignore">
      <header class="kith-sheet-head">
        <Dialog.Title class="t-title">{title}</Dialog.Title>
        <Dialog.Close>
          {#snippet child({ props })}
            <IconButton icon="x" label={closeLabel} triggerProps={props} />
          {/snippet}
        </Dialog.Close>
      </header>
      <div class="kith-sheet-body">{@render children?.()}</div>
      {#if footer}<footer class="kith-sheet-foot">{@render footer()}</footer>{/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  :global(.kith-sheet) {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: var(--z-dialog);
    width: var(--sheet);
    max-width: 100vw;
    display: grid;
    grid-template-rows: auto 1fr auto;
    background: var(--surface);
    color: var(--ink-1);
    box-shadow: var(--shadow-pop);
  }
  :global(.kith-sheet[data-state='open']) {
    animation: kith-sheet-in var(--d-sheet) var(--e-sheet);
  }
  :global(.kith-sheet-head) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    padding: var(--s-4) var(--s-5);
    border-bottom: var(--hairline) solid var(--line);
  }
  :global(.kith-sheet-body) {
    overflow-y: auto;
    padding: var(--s-5);
    display: grid;
    align-content: start;
    gap: var(--s-4);
  }
  :global(.kith-sheet-foot) {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-3);
    padding: var(--s-4) var(--s-5);
    padding-bottom: calc(var(--s-4) + env(safe-area-inset-bottom));
    border-top: var(--hairline) solid var(--line);
  }
  @media (max-width: 40rem) {
    :global(.kith-sheet) {
      width: 100vw;
    }
    :global(.kith-sheet-head),
    :global(.kith-sheet-body),
    :global(.kith-sheet-foot) {
      padding-inline: var(--gutter-mobile);
    }
  }
  @keyframes -global-kith-sheet-in {
    from {
      transform: translateX(100%);
    }
  }
</style>
