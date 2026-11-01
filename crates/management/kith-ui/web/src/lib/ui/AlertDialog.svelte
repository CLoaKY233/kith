<!--
@component
AlertDialog (spec §5, §7.9, §13 tiers 2 and 3): consequences first, then [Cancel] [Action].
Focus starts on Cancel and Enter is never bound to the action; nothing is typed unless the
caller adds a type-to-confirm field. A click on the scrim does nothing; Esc cancels. While
`pending`, Cancel and Esc do nothing: the op is being signed and the outcome shows here.
Returns focus to the element that opened it, as Dialog does.
Props:
  - open: boolean, bindable.
  - title: string (from copy.ts), the question: "Remove phone from home?".
  - description?: string, the first consequence line, in `--ink-2`.
  - children?: Snippet, more body: consequence lines, a checkbox, a typed confirmation.
  - cancelLabel: string. confirmLabel: string, naming the object ("Remove phone").
  - confirmVariant?: 'danger' | 'primary' (default 'danger').
  - confirmDisabled?: boolean, only for a type-to-confirm field.
  - pending?: boolean, the action is running (Button shows its spinner after 150 ms).
  - status?: string, a progress line above the buttons in `--ink-3`
    (`Waiting for confirmation on this computer…`). Announced through announce().
  - error?: string, why the action failed, in `--danger` above the buttons. Announced too.
  - onConfirm: () => void.
  - onOpenChange?: (open: boolean) => void.
-->
<script lang="ts">
  import { AlertDialog } from 'bits-ui';
  import { tick, type Snippet } from 'svelte';
  import { announce } from '../announce';
  import Button from './Button.svelte';
  import { modalLayer } from './layers.svelte';
  import './overlay.css';

  let {
    open = $bindable(false),
    title,
    description,
    children,
    cancelLabel,
    confirmLabel,
    confirmVariant = 'danger',
    confirmDisabled = false,
    pending = false,
    status,
    error,
    onConfirm,
    onOpenChange,
  }: {
    open?: boolean;
    title: string;
    description?: string;
    children?: Snippet;
    cancelLabel: string;
    confirmLabel: string;
    confirmVariant?: 'danger' | 'primary';
    confirmDisabled?: boolean;
    pending?: boolean;
    status?: string;
    error?: string;
    onConfirm: () => void;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const layer = modalLayer(() => open);
  let cancel: HTMLButtonElement | null = $state(null);

  // The announcer is the page's one polite live region (§11.1).
  $effect(() => {
    const line = error ?? status;
    if (open && line) announce(line);
  });

  function close(): void {
    open = false;
    onOpenChange?.(false);
  }
</script>

<AlertDialog.Root bind:open {onOpenChange}>
  <AlertDialog.Portal>
    <AlertDialog.Overlay class="kith-scrim" />
    <AlertDialog.Content
      class="kith-dialog"
      data-kith-layer={layer.id}
      escapeKeydownBehavior={pending ? 'ignore' : 'close'}
      onOpenAutoFocus={(event) => {
        event.preventDefault();
        void tick().then(() => cancel?.focus());
      }}
      onCloseAutoFocus={layer.onCloseAutoFocus}
    >
      <div class="kith-dialog-titles">
        <AlertDialog.Title level={2} class="kith-dialog-title">{title}</AlertDialog.Title>
        {#if description}
          <AlertDialog.Description class="kith-dialog-desc">{description}</AlertDialog.Description>
        {/if}
      </div>
      {#if children}<div class="kith-dialog-body">{@render children()}</div>{/if}
      {#if error}
        <p class="kith-dialog-status danger">{error}</p>
      {:else if status}
        <p class="kith-dialog-status">{status}</p>
      {/if}
      <div class="kith-dialog-footer">
        <Button bind:ref={cancel} disabled={pending} onclick={close}>{cancelLabel}</Button>
        <Button
          variant={confirmVariant}
          disabled={confirmDisabled}
          {pending}
          onclick={() => onConfirm()}
        >
          {confirmLabel}
        </Button>
      </div>
    </AlertDialog.Content>
  </AlertDialog.Portal>
</AlertDialog.Root>
