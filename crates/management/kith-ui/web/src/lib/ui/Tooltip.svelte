<!--
@component
Tooltip (spec §5): 400 ms delay, never essential information, skipped on touch.
App.svelte wraps the page in one `Tooltip.Provider`.
Props:
  - text: string (from copy.ts)
  - trigger: Snippet<[Record<string, unknown>]>. Spread the given props on the element the
    tooltip describes.
  - side?: 'top' | 'right' | 'bottom' | 'left' (default 'top')
-->
<script lang="ts">
  import { Tooltip } from 'bits-ui';
  import type { Snippet } from 'svelte';
  import { TOOLTIP_DELAY_MS } from '../live/timing';

  let {
    text,
    trigger,
    side = 'top',
  }: {
    text: string;
    trigger: Snippet<[Record<string, unknown>]>;
    side?: 'top' | 'right' | 'bottom' | 'left';
  } = $props();

  const coarse =
    typeof window !== 'undefined' && window.matchMedia('(pointer: coarse)').matches;
</script>

{#if coarse}
  {@render trigger({})}
{:else}
  <Tooltip.Root delayDuration={TOOLTIP_DELAY_MS}>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        {@render trigger(props)}
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Portal>
      <Tooltip.Content {side} sideOffset={6} class="kith-tooltip">{text}</Tooltip.Content>
    </Tooltip.Portal>
  </Tooltip.Root>
{/if}

<style>
  :global(.kith-tooltip) {
    z-index: var(--z-popover);
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-1);
    background: var(--ink-1);
    color: var(--surface);
    font: var(--t-small);
    box-shadow: var(--shadow-pop);
  }
</style>
