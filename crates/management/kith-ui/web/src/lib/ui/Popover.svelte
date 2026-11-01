<!--
@component
Popover (spec §5): Bits Popover with `--shadow-pop`, in 140 ms, out 100 ms.
Props:
  - open?: boolean, bindable.
  - trigger: Snippet<[Record<string, unknown>]>. Spread the props on the trigger element.
  - children: the content.
  - label?: string, the content's accessible name when it has no heading.
  - side?: 'top' | 'right' | 'bottom' | 'left' (default 'bottom'); align?: 'start' | 'center' | 'end' (default 'start').
  - width?: 'auto' | 'card' (card = 360 px, for NeedsYouCard lists and focus cards).
-->
<script lang="ts">
  import { Popover } from 'bits-ui';
  import type { Snippet } from 'svelte';

  let {
    open = $bindable(false),
    trigger,
    children,
    label,
    side = 'bottom',
    align = 'start',
    width = 'auto',
  }: {
    open?: boolean;
    trigger: Snippet<[Record<string, unknown>]>;
    children: Snippet;
    label?: string;
    side?: 'top' | 'right' | 'bottom' | 'left';
    align?: 'start' | 'center' | 'end';
    width?: 'auto' | 'card';
  } = $props();
</script>

<Popover.Root bind:open>
  <Popover.Trigger>
    {#snippet child({ props })}
      {@render trigger(props)}
    {/snippet}
  </Popover.Trigger>
  <Popover.Portal>
    <Popover.Content
      {side}
      {align}
      sideOffset={8}
      class="kith-pop {width}"
      aria-label={label}
    >
      {@render children()}
    </Popover.Content>
  </Popover.Portal>
</Popover.Root>

<style>
  :global(.kith-pop) {
    z-index: var(--z-popover);
    padding: var(--s-3);
    border-radius: var(--r-3);
    border: var(--hairline) solid var(--line);
    background: var(--surface);
    color: var(--ink-1);
    box-shadow: var(--shadow-pop);
  }
  :global(.kith-pop.card) {
    width: var(--recent-column);
  }
  :global(.kith-pop[data-state='open']) {
    animation: kith-pop-in var(--d-pop-in) var(--e-pop);
  }
  :global(.kith-pop[data-state='closed']) {
    animation: kith-pop-out var(--d-pop-out) var(--e-pop);
  }
  @keyframes -global-kith-pop-in {
    from {
      opacity: 0;
    }
  }
  @keyframes -global-kith-pop-out {
    to {
      opacity: 0;
    }
  }
</style>
