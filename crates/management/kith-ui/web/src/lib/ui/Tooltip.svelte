<!--
@component
Tooltip (spec §5): shows after 400 ms of hover or on keyboard focus, in 12.5 px text on an
inverted surface. It never carries essential information: the same text is always the
element's accessible name or visible elsewhere. On touch (pointer: coarse) it is skipped and
the trigger renders bare. The pointer cannot move into it; it closes on pointer down, Esc or
blur. Inside a modal it stacks above it. App.svelte wraps the page in one `Tooltip.Provider`.
Props:
  - text: string (from copy.ts).
  - trigger: Snippet<[Record<string, unknown>]>. Spread the given props on the element the
    tooltip describes.
  - side?: 'top' | 'right' | 'bottom' | 'left' (default 'top').
  - id?: string, the trigger's id. When the trigger is also another primitive's trigger (a
    DropdownMenu or Popover), pass that primitive's trigger id here: both look the element
    up by id, so they must share one.
-->
<script lang="ts">
  import { Tooltip } from 'bits-ui';
  import type { Snippet } from 'svelte';
  import { TOOLTIP_DELAY_MS } from '../live/timing';
  import { coarse, FLOAT_OFFSET, insideModal } from './layers.svelte';
  import './overlay.css';

  let {
    text,
    trigger,
    side = 'top',
    id,
  }: {
    text: string;
    trigger: Snippet<[Record<string, unknown>]>;
    side?: 'top' | 'right' | 'bottom' | 'left';
    id?: string;
  } = $props();

  const modal = insideModal();
</script>

{#if coarse.current}
  {@render trigger({})}
{:else}
  <Tooltip.Root delayDuration={TOOLTIP_DELAY_MS} disableHoverableContent ignoreNonKeyboardFocus>
    <Tooltip.Trigger {id}>
      {#snippet child({ props })}
        {@render trigger(props)}
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Portal>
      <Tooltip.Content
        {side}
        sideOffset={FLOAT_OFFSET}
        collisionPadding={FLOAT_OFFSET}
        escapeKeydownBehavior="defer-otherwise-close"
        class="kith-float kith-tip {modal ? 'in-modal' : ''}"
      >
        {text}
      </Tooltip.Content>
    </Tooltip.Portal>
  </Tooltip.Root>
{/if}

<style>
  :global(.kith-tip) {
    max-width: min(var(--measure), calc(100vw - 2 * var(--gutter-mobile)));
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-2);
    border-color: var(--ink-1);
    background: var(--ink-1);
    color: var(--surface);
    font: var(--t-small);
    letter-spacing: var(--t-small-tracking);
    pointer-events: none;
  }
</style>
