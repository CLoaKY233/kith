<!--
@component
CopyField (spec §5): a mono value on `--sunken` plus `Copy`, which becomes `Copied` with
a check icon for 1.5 s.
Props:
  - value: string, the exact text copied.
  - label?: string, an accessible name for the value (for example "Invite link").
  - onCopy?: () => void.
-->
<script lang="ts">
  import { copy } from '../copy';
  import Icon from '../icons/Icon.svelte';
  import { COPIED_MS } from '../live/timing';

  let { value, label, onCopy }: { value: string; label?: string; onCopy?: () => void } =
    $props();

  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function write(): Promise<void> {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      return;
    }
    copied = true;
    onCopy?.();
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => (copied = false), COPIED_MS);
  }
</script>

<div class="copy-field">
  <code class="value" aria-label={label}>{value}</code>
  <button type="button" class="action" onclick={write}>
    {#if copied}
      <Icon name="check" />
      <span>{copy.copyField.copied}</span>
    {:else}
      <span>{copy.copyField.copy}</span>
    {/if}
  </button>
  <span class="sr-only" aria-live="polite">{copied ? copy.copyField.copied : ''}</span>
</div>

<style>
  .copy-field {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--control-coarse);
    padding: 0 var(--s-1) 0 var(--s-3);
    border-radius: var(--r-2);
    background: var(--sunken);
  }
  .value {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    font: var(--t-mono);
    color: var(--ink-1);
  }
  .action {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--s-1);
    min-width: calc(var(--s-8) + var(--s-3));
    height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    font-weight: 600;
    color: var(--ink-1);
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .action:hover {
    background: var(--surface);
  }
</style>
