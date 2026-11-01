<!--
@component
AlertDialog (spec §5, §7.9, §13 tier 2): consequences first, focus starts on Cancel,
Enter is never bound to the action. The action button names the object ("Remove phone").
Props:
  - open: boolean, bindable.
  - title: string (from copy.ts).
  - description?: string.
  - children?: Snippet, extra body (consequence lines, a checkbox, a typed confirmation).
  - cancelLabel: string. confirmLabel: string.
  - confirmVariant?: 'danger' | 'primary' (default 'danger').
  - confirmDisabled?: boolean, only for a type-to-confirm field.
  - pending?: boolean, the action is running.
  - onConfirm: () => void.
-->
<script lang="ts">
  import { AlertDialog } from 'bits-ui';
  import type { Snippet } from 'svelte';
  import Button from './Button.svelte';

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
    onConfirm,
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
    onConfirm: () => void;
  } = $props();

  let cancel: HTMLButtonElement | null = $state(null);
</script>

<AlertDialog.Root bind:open>
  <AlertDialog.Portal>
    <AlertDialog.Overlay class="kith-scrim" />
    <AlertDialog.Content
      class="kith-dialog"
      onOpenAutoFocus={(event) => {
        event.preventDefault();
        cancel?.focus();
      }}
    >
      <AlertDialog.Title class="t-heading">{title}</AlertDialog.Title>
      {#if description}
        <AlertDialog.Description class="kith-dialog-desc">{description}</AlertDialog.Description>
      {/if}
      {@render children?.()}
      <div class="kith-dialog-footer">
        <AlertDialog.Cancel>
          {#snippet child({ props })}
            <Button {...props} bind:ref={cancel}>{cancelLabel}</Button>
          {/snippet}
        </AlertDialog.Cancel>
        <Button variant={confirmVariant} disabled={confirmDisabled} {pending} onclick={onConfirm}>
          {confirmLabel}
        </Button>
      </div>
    </AlertDialog.Content>
  </AlertDialog.Portal>
</AlertDialog.Root>
