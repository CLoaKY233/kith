<!--
@component
Segmented (spec §5): a Bits ToggleGroup in single mode. 32 px, `--sunken` track, the
selected segment on `--surface` with a hairline. Always one selected. A disabled segment
carries an inline reason under the control.
Props:
  - value: string, bindable.
  - options: { value: string; label: string; disabled?: boolean; reason?: string }[]
  - label: string, the group's accessible name (from copy.ts).
  - onValueChange?: (value: string) => void.
-->
<script lang="ts" module>
  export type SegmentOption = {
    value: string;
    label: string;
    disabled?: boolean;
    reason?: string;
  };
</script>

<script lang="ts">
  import { ToggleGroup } from 'bits-ui';

  let {
    value = $bindable(''),
    options,
    label,
    onValueChange,
  }: {
    value?: string;
    options: readonly SegmentOption[];
    label: string;
    onValueChange?: (value: string) => void;
  } = $props();

  const reasons = $derived(options.filter((o) => o.disabled && o.reason));

  function change(next: string): void {
    if (next === '' || next === value) return;
    value = next;
    onValueChange?.(next);
  }
</script>

<div class="segmented">
  <ToggleGroup.Root type="single" {value} onValueChange={change} aria-label={label} class="track">
    {#each options as option (option.value)}
      <ToggleGroup.Item value={option.value} disabled={option.disabled} class="segment">
        {option.label}
      </ToggleGroup.Item>
    {/each}
  </ToggleGroup.Root>
  {#each reasons as option (option.value)}
    <p class="reason">{option.reason}</p>
  {/each}
</div>

<style>
  .segmented {
    display: inline-grid;
    gap: var(--s-1);
  }
  .segmented :global(.track) {
    display: inline-flex;
    height: var(--control);
    padding: calc(var(--s-1) / 2);
    gap: 0;
    border-radius: var(--r-2);
    background: var(--sunken);
  }
  .segmented :global(.segment) {
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    border: var(--hairline) solid transparent;
    color: var(--ink-2);
    font: var(--t-body);
    cursor: pointer;
    transition: color var(--d-hover) var(--e-linear);
  }
  .segmented :global(.segment[data-state='on']) {
    background: var(--surface);
    border-color: var(--line);
    color: var(--ink-1);
    font-weight: 600;
  }
  .segmented :global(.segment[data-disabled]) {
    color: var(--ink-3);
    cursor: default;
  }
  .reason {
    font: var(--t-small);
    color: var(--ink-3);
  }
</style>
