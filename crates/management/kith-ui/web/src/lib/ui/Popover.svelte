<!--
@component
Popover (spec §5): Bits Popover on `--surface` with a hairline, `--r-3` and `--shadow-pop`.
Fades in over 140 ms and out over 100 ms (instant under reduced motion). Esc or a click
outside closes it and focus returns to the trigger. Inside a modal it stacks above it.
Used for the Needs-you list, the RoleChip, the ExitPicker, map focus cards and, on desktop,
Allow this… anchored to its card (§7.5).
Props:
  - open?: boolean, bindable.
  - trigger?: Snippet<[Record<string, unknown>]>. Spread the props on the trigger element.
    Leave it out to open the popover from code, against `anchor`.
  - anchor?: HTMLElement | null, the element to position against instead of the trigger
    (the card for Allow this…).
  - children: the content.
  - label?: string, the content's accessible name when it has no heading.
  - side?: 'top' | 'right' | 'bottom' | 'left' (default 'bottom'); align?: 'start' | 'center' | 'end' (default 'start').
  - width?: 'auto' | 'card' (card = 360 px, for NeedsYouCard lists and focus cards).
  - flush?: boolean. No inner padding, for content that brings its own (a list of cards).
  - onOpenChange?: (open: boolean) => void.
  - onOpenAutoFocus?: (event: Event) => void. Call preventDefault() and focus something
    else to keep focus off the first field (a NeedsYouCard's name input).
-->
<script lang="ts">
  import { Popover } from 'bits-ui';
  import type { Snippet } from 'svelte';
  import { FLOAT_OFFSET, insideModal } from './layers.svelte';
  import './overlay.css';

  let {
    open = $bindable(false),
    trigger,
    anchor = null,
    children,
    label,
    side = 'bottom',
    align = 'start',
    width = 'auto',
    flush = false,
    onOpenChange,
    onOpenAutoFocus,
  }: {
    open?: boolean;
    trigger?: Snippet<[Record<string, unknown>]>;
    anchor?: HTMLElement | null;
    children: Snippet;
    label?: string;
    side?: 'top' | 'right' | 'bottom' | 'left';
    align?: 'start' | 'center' | 'end';
    width?: 'auto' | 'card';
    flush?: boolean;
    onOpenChange?: (open: boolean) => void;
    onOpenAutoFocus?: (event: Event) => void;
  } = $props();

  const modal = insideModal();
</script>

<Popover.Root bind:open {onOpenChange}>
  {#if trigger}
    <Popover.Trigger>
      {#snippet child({ props })}
        {@render trigger(props)}
      {/snippet}
    </Popover.Trigger>
  {/if}
  <Popover.Portal>
    <Popover.Content
      {side}
      {align}
      sideOffset={FLOAT_OFFSET}
      collisionPadding={FLOAT_OFFSET}
      customAnchor={anchor}
      class="kith-float kith-pop {width} {modal ? 'in-modal' : ''}"
      data-flush={flush ? '' : undefined}
      aria-label={label}
      {onOpenAutoFocus}
    >
      {@render children()}
    </Popover.Content>
  </Popover.Portal>
</Popover.Root>

<style>
  :global(.kith-pop) {
    max-width: calc(100vw - 2 * var(--gutter-mobile));
    max-height: var(--bits-floating-available-height);
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: var(--s-3);
  }
  :global(.kith-pop[data-flush]) {
    padding: 0;
  }
  :global(.kith-pop.card) {
    width: var(--pop-card);
  }
</style>
