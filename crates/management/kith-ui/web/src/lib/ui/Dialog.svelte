<!--
@component
Dialog (spec §5): Bits Dialog, max width 440, `--r-3`, `--shadow-modal`, plain scrim. Opens
with opacity and scale .97→1 over 180 ms (instant under reduced motion). Esc and a click on
the scrim close it. Focus moves to the first control inside and returns to the element that
opened it, also when that was a menu item or the open came from the URL. A dialog with no
control in its body starts on the dialog itself. No modal ever opens
from another modal: in development a nested open logs a warning.
Props:
  - open: boolean, bindable.
  - title: string (from copy.ts).
  - description?: string, under the title in `--ink-2`; wired as aria-describedby.
  - children?: Snippet, the body.
  - footer?: Snippet, the buttons row: right-aligned, 12 px apart, primary last.
  - closeLabel?: string. When given, an × IconButton with this name sits beside the title.
    Give it to dialogs without a Cancel button.
  - onOpenChange?: (open: boolean) => void. Screens map this to the URL's overlay param.
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
    onOpenChange,
  }: {
    open?: boolean;
    title: string;
    description?: string;
    children?: Snippet;
    footer?: Snippet;
    closeLabel?: string;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const layer = modalLayer(() => open);
  let content: HTMLElement | null = $state(null);

  function close(): void {
    open = false;
    onOpenChange?.(false);
  }

  /**
   * Starts on the first control of the body or footer. A dialog with only text starts on
   * the dialog itself, so it is read from the title, and never on the × beside the title.
   */
  function focusFirst(event: Event): void {
    event.preventDefault();
    void tick().then(() => {
      const target =
        firstTabbable(content?.querySelector('.kith-dialog-body') ?? null) ??
        firstTabbable(content?.querySelector('.kith-dialog-footer') ?? null) ??
        content;
      target?.focus();
    });
  }
</script>

<Dialog.Root bind:open {onOpenChange}>
  <Dialog.Portal>
    <Dialog.Overlay class="kith-scrim" />
    <Dialog.Content
      bind:ref={content}
      class="kith-dialog"
      data-kith-layer={layer.id}
      onOpenAutoFocus={focusFirst}
      onCloseAutoFocus={layer.onCloseAutoFocus}
    >
      <div class="kith-dialog-head">
        <div class="kith-dialog-titles">
          <Dialog.Title level={2} class="kith-dialog-title">{title}</Dialog.Title>
          {#if description}
            <Dialog.Description class="kith-dialog-desc">{description}</Dialog.Description>
          {/if}
        </div>
        {#if closeLabel}
          <IconButton icon="x" label={closeLabel} onclick={close} />
        {/if}
      </div>
      {#if children}<div class="kith-dialog-body">{@render children()}</div>{/if}
      {#if footer}<div class="kith-dialog-footer">{@render footer()}</div>{/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>
