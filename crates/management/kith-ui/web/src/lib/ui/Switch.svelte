<!--
@component
Switch: a Bits Switch with a visible label on the left and the control on the right, in
a row of `--row` height. Off: `--sunken` track with a `--line-strong` border (3:1, WCAG
1.4.11) and an `--ink-3` thumb. On: `--accent` track with an `--accent-fg` thumb. Clicking
the label toggles it; Space and Enter toggle it from the keyboard.
Props:
  - checked: boolean, bindable.
  - label: string (from copy.ts).
  - helper?: string, under the label in `--ink-3` (the privacy switch's explanation).
  - hideLabel?: boolean, keep the label for assistive tech only (a switch alone in a table row).
  - disabled?: boolean. `aria-disabled` (a stale connection): stays focusable, does not
    toggle. Pair it with inline reason text.
  - onCheckedChange?: (checked: boolean) => void, called only for a real change.
-->
<script lang="ts">
  import { Switch } from 'bits-ui';

  let {
    checked = $bindable(false),
    label,
    helper,
    hideLabel = false,
    disabled = false,
    onCheckedChange,
  }: {
    checked?: boolean;
    label: string;
    helper?: string;
    hideLabel?: boolean;
    disabled?: boolean;
    onCheckedChange?: (checked: boolean) => void;
  } = $props();

  const id = $props.id();

  function set(next: boolean): void {
    if (disabled || next === checked) return;
    checked = next;
    onCheckedChange?.(next);
  }
</script>

<div class="switch-row" class:bare={hideLabel}>
  <div class="text">
    <label for={id} class:sr-only={hideLabel}>{label}</label>
    {#if helper}<p class="helper" id={`${id}-help`}>{helper}</p>{/if}
  </div>
  <Switch.Root
    {id}
    bind:checked={() => checked, set}
    aria-disabled={disabled ? 'true' : undefined}
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
  .switch-row.bare {
    justify-content: flex-end;
    min-height: 0;
  }
  .text {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  label {
    cursor: pointer;
  }
  .helper {
    font: var(--t-small);
    color: var(--ink-3);
    max-width: var(--measure);
  }
  .switch-row :global(.kith-switch) {
    flex: none;
    position: relative;
    display: inline-block;
    width: var(--switch-w);
    height: var(--switch-h);
    padding: 0;
    border: var(--hairline) solid var(--line-strong);
    border-radius: var(--r-full);
    background: var(--sunken);
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    transition:
      background-color var(--d-hover) var(--e-linear),
      border-color var(--d-hover) var(--e-linear);
  }
  .switch-row :global(.kith-switch:focus-visible) {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .switch-row :global(.kith-switch:not([aria-disabled='true']):hover) {
    border-color: var(--ink-3);
  }
  .switch-row :global(.kith-switch[data-state='checked']) {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch-row :global(.kith-switch[data-state='checked']:not([aria-disabled='true']):hover) {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }
  .switch-row :global(.kith-switch[aria-disabled='true']) {
    cursor: default;
    border-color: var(--line);
  }
  .switch-row :global(.kith-switch[aria-disabled='true'][data-state='checked']) {
    background: var(--ink-4);
  }
  /* The thumb sits centred in the track, inset by the same gap on every side. */
  .switch-row :global(.kith-switch-thumb) {
    position: absolute;
    inset-block: calc((var(--switch-h) - var(--switch-thumb)) / 2 - var(--hairline));
    inset-inline-start: calc((var(--switch-h) - var(--switch-thumb)) / 2 - var(--hairline));
    display: block;
    width: var(--switch-thumb);
    height: var(--switch-thumb);
    border-radius: var(--r-full);
    background: var(--ink-3);
    transition:
      transform var(--d-pop-in) var(--e-pop),
      background-color var(--d-hover) var(--e-linear);
  }
  .switch-row :global(.kith-switch-thumb[data-state='checked']) {
    background: var(--accent-fg);
    transform: translateX(
      calc(var(--switch-w) - var(--switch-h))
    );
  }
  /* Touch: a 44×44 hit area around the 40×24 control (§11.4). */
  @media (pointer: coarse) {
    .switch-row :global(.kith-switch)::after {
      content: '';
      position: absolute;
      inset: calc((var(--switch-h) - var(--target-touch)) / 2) calc((var(--switch-w) - var(--target-touch)) / 2);
    }
  }
  .switch-row:has(:global(.kith-switch[aria-disabled='true'])) label {
    cursor: default;
  }
  @media (forced-colors: active) {
    .switch-row :global(.kith-switch) {
      border-color: ButtonText;
      forced-color-adjust: none;
      background: Canvas;
    }
    .switch-row :global(.kith-switch[data-state='checked']) {
      background: Highlight;
      border-color: Highlight;
    }
    .switch-row :global(.kith-switch-thumb) {
      forced-color-adjust: none;
      background: ButtonText;
    }
    .switch-row :global(.kith-switch-thumb[data-state='checked']) {
      background: HighlightText;
    }
  }
</style>
