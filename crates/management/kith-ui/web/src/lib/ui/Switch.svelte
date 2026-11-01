<!--
@component
Switch: Bits Switch with a visible label. Border `--line-strong` (3:1, WCAG 1.4.11).
Props:
  - checked: boolean, bindable.
  - label: string (from copy.ts).
  - helper?: string, under the label in `--ink-3`.
  - disabled?: boolean.
  - onCheckedChange?: (checked: boolean) => void.
-->
<script lang="ts">
  import { Switch } from 'bits-ui';

  let {
    checked = $bindable(false),
    label,
    helper,
    disabled = false,
    onCheckedChange,
  }: {
    checked?: boolean;
    label: string;
    helper?: string;
    disabled?: boolean;
    onCheckedChange?: (checked: boolean) => void;
  } = $props();

  const id = $props.id();
</script>

<div class="switch-row">
  <div class="text">
    <label for={id}>{label}</label>
    {#if helper}<p class="helper" id={`${id}-help`}>{helper}</p>{/if}
  </div>
  <Switch.Root
    {id}
    bind:checked
    {disabled}
    {onCheckedChange}
    aria-describedby={helper ? `${id}-help` : undefined}
    class="kith-switch"
  >
    <Switch.Thumb class="kith-switch-thumb" />
  </Switch.Root>
</div>

<style>
  .switch-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
    min-height: var(--row);
  }
  .text {
    display: grid;
    gap: var(--s-1);
  }
  .helper {
    font: var(--t-small);
    color: var(--ink-3);
    max-width: var(--measure);
  }
  .switch-row :global(.kith-switch) {
    flex: none;
    display: inline-flex;
    align-items: center;
    width: calc(var(--s-6) + var(--s-2));
    height: var(--s-5);
    padding: calc(var(--s-1) / 2);
    border-radius: var(--r-full);
    border: var(--hairline) solid var(--line-strong);
    background: var(--sunken);
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .switch-row :global(.kith-switch[data-state='checked']) {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch-row :global(.kith-switch-thumb) {
    display: block;
    width: var(--s-4);
    height: var(--s-4);
    margin-inline: calc(var(--s-1) / 2);
    border-radius: var(--r-full);
    background: var(--surface);
    transition: transform var(--d-hover) var(--e-linear);
  }
  .switch-row :global(.kith-switch-thumb[data-state='checked']) {
    transform: translateX(var(--s-4));
  }
</style>
