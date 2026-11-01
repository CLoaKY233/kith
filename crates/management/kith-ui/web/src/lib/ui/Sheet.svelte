<!--
@component
Sheet (spec §4.4, §5): the container for flows. On desktop a 480 px panel slides in from the
right over the scrim (240 ms, `--e-sheet`); below 40rem it is a full-screen page. The title
row sits at the top, the body scrolls, and the footer with the primary action is pinned at
the bottom (with the safe-area inset on phones). A sheet that is part of a flow stays open
until the outcome or the user cancels: a click on the scrim does nothing, while Esc and the
× close it. Focus starts on the first control of the body (on the sheet itself when the body
has none, so it is read from the title) and returns to the element that opened it.
Props:
  - open: boolean, bindable.
  - title: string (from copy.ts).
  - description?: string, under the title in `--ink-2`; wired as aria-describedby.
  - children?: Snippet, the body (scrolls).
  - footer?: Snippet, pinned at the bottom: buttons right-aligned 12 px apart, primary last.
    On phones they share the width.
  - closeLabel: string, the × button's accessible name and tooltip.
  - autoCloseMs?: number | null. When set, the sheet closes after this long while neither
    the pointer nor keyboard focus is inside it (§7.3: 2 s after the device is online). The
    count restarts each time the pointer or keyboard focus leaves. Focus that stayed on a
    clicked button does not hold the sheet open.
  - onOpenChange?: (open: boolean) => void. Screens map this to the `?add=` URL param.
-->
<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { tick, type Snippet } from 'svelte';
  import IconButton from './IconButton.svelte';
  import { firstTabbable, modalLayer } from './layers.svelte';
  import './overlay.css';

  let {
    open = $bindable(false),
    title,
    description,
    children,
    footer,
    closeLabel,
    autoCloseMs = null,
    onOpenChange,
  }: {
    open?: boolean;
    title: string;
    description?: string;
    children?: Snippet;
    footer?: Snippet;
    closeLabel: string;
    autoCloseMs?: number | null;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const layer = modalLayer(() => open);
  let content: HTMLElement | null = $state(null);
  let body: HTMLElement | null = $state(null);
  let pointerInside = $state(false);
  let focusInside = $state(false);

  $effect(() => {
    if (!open) {
      pointerInside = false;
      focusInside = false;
    }
  });

  $effect(() => {
    if (!open || autoCloseMs === null || pointerInside || focusInside) return;
    const timer = setTimeout(() => {
      // The sheet may have slid in under a pointer that has not moved since.
      if (content?.matches(':hover')) {
        pointerInside = true;
        return;
      }
      close();
    }, autoCloseMs);
    return () => clearTimeout(timer);
  });

  /**
   * Focus is always inside a modal sheet, so only a control the person moved to with the
   * keyboard counts as being in use; focus left on a clicked button does not.
   */
  function engaged(target: EventTarget | null): boolean {
    return target instanceof HTMLElement && target !== content && target.matches(':focus-visible');
  }

  function close(): void {
    open = false;
    onOpenChange?.(false);
  }

  function focusFirst(event: Event): void {
    event.preventDefault();
    void tick().then(() => (firstTabbable(body) ?? content)?.focus());
  }
</script>

<Dialog.Root bind:open {onOpenChange}>
  <Dialog.Portal>
    <Dialog.Overlay class="kith-scrim" />
    <Dialog.Content
      bind:ref={content}
      class="kith-sheet"
      data-kith-layer={layer.id}
      interactOutsideBehavior="ignore"
      onOpenAutoFocus={focusFirst}
      onCloseAutoFocus={layer.onCloseAutoFocus}
      onpointerenter={() => (pointerInside = true)}
      onpointerleave={() => (pointerInside = false)}
      onfocusin={(event: FocusEvent) => (focusInside = engaged(event.target))}
      onfocusout={(event: FocusEvent) => {
        if (!(event.relatedTarget instanceof Node && content?.contains(event.relatedTarget))) {
          focusInside = false;
        }
      }}
    >
      <header class="kith-sheet-head">
        <div class="kith-dialog-titles">
          <Dialog.Title level={2} class="kith-sheet-title">{title}</Dialog.Title>
          {#if description}
            <Dialog.Description class="kith-dialog-desc">{description}</Dialog.Description>
          {/if}
        </div>
        <IconButton icon="x" label={closeLabel} onclick={close} />
      </header>
      <div class="kith-sheet-body" bind:this={body}>{@render children?.()}</div>
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
    width: min(var(--sheet), 100vw);
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    background: var(--surface);
    color: var(--ink-1);
    box-shadow: var(--shadow-pop);
    outline: none;
  }
  :global(.kith-sheet-head) {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--s-3);
    padding: var(--s-4) var(--s-5);
    border-bottom: var(--hairline) solid var(--line);
  }
  :global(.kith-sheet-title) {
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
    overflow-wrap: anywhere;
  }
  :global(.kith-sheet-body) {
    overflow-y: auto;
    overscroll-behavior: contain;
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
    background: var(--surface);
  }
  @media (width < 40rem) {
    :global(.kith-sheet) {
      width: 100vw;
      box-shadow: none;
    }
    :global(.kith-sheet-head) {
      padding: var(--s-3) var(--gutter-mobile);
      padding-top: calc(var(--s-3) + env(safe-area-inset-top));
    }
    :global(.kith-sheet-body) {
      padding: var(--s-4) var(--gutter-mobile);
    }
    :global(.kith-sheet-foot) {
      padding-inline: var(--gutter-mobile);
    }
    :global(.kith-sheet-foot > *) {
      flex: 1 1 0;
    }
  }
  @media (forced-colors: active) {
    :global(.kith-sheet) {
      border-left: var(--hairline) solid CanvasText;
    }
  }
  @media (prefers-reduced-motion: no-preference) {
    :global(.kith-sheet[data-state='open']) {
      animation: kith-sheet-in var(--d-sheet) var(--e-sheet);
    }
    :global(.kith-sheet[data-state='closed']) {
      animation: kith-sheet-out var(--d-sheet) var(--e-sheet);
    }
  }
  @keyframes -global-kith-sheet-in {
    from {
      transform: translateX(100%);
    }
  }
  @keyframes -global-kith-sheet-out {
    to {
      transform: translateX(100%);
    }
  }
</style>
