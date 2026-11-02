<!--
@component
AlertDialog (spec §5, §7.9, §13 tiers 2 and 3): consequences first, then [Cancel] [Action].
Focus starts on Cancel (or on `initialFocus`, a type-to-confirm field) and Enter is never
bound to the action; nothing is typed unless the caller adds a type-to-confirm field. A click on the scrim does nothing; Esc cancels. While
`pending`, Cancel (aria-disabled, described by the status line) and Esc do nothing: the op
is being signed and the outcome shows here.
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
  - initialFocus?: HTMLElement | null, focused on open instead of Cancel.
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
    initialFocus = null,
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
    initialFocus?: HTMLElement | null;
    onConfirm: () => void;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const layer = modalLayer(() => open);
  let cancel: HTMLButtonElement | null = $state(null);
  const id = $props.id();
  const descId = `${id}-desc`;
  const bodyId = `${id}-body`;
  const statusId = `${id}-status`;
  const describedBy = $derived(
    [description ? descId : '', children ? bodyId : ''].filter(Boolean).join(' ') || undefined,
  );

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
        void tick().then(() => (initialFocus ?? cancel)?.focus());
      }}
      onCloseAutoFocus={layer.onCloseAutoFocus}
    >
      {#snippet child({ props })}
        <!-- Every consequence line is read with the title, not only the first (§13 tier 2). -->
        <div {...props} aria-describedby={describedBy}>
          <div class="kith-dialog-titles">
            <AlertDialog.Title level={2} class="kith-dialog-title">{title}</AlertDialog.Title>
            {#if description}
              <AlertDialog.Description id={descId} class="kith-dialog-desc">
                {description}
              </AlertDialog.Description>
            {/if}
          </div>
          {#if children}<div class="kith-dialog-body" id={bodyId}>{@render children()}</div>{/if}
          {#if error}
            <p class="kith-dialog-status danger" id={statusId}>{error}</p>
          {:else if status}
            <p class="kith-dialog-status" id={statusId}>{status}</p>
          {/if}
          <div class="kith-dialog-footer">
            <Button
              bind:ref={cancel}
              disabled={pending}
              aria-describedby={pending && (error ?? status) ? statusId : undefined}
              onclick={close}
            >
              {cancelLabel}
            </Button>
            <Button
              variant={confirmVariant}
              disabled={confirmDisabled}
              {pending}
              onclick={() => onConfirm()}
            >
              {confirmLabel}
            </Button>
          </div>
        </div>
      {/snippet}
    </AlertDialog.Content>
  </AlertDialog.Portal>
</AlertDialog.Root>
