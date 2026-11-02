<!--
@component
PathCard: PathBadge's hover card, loaded after the first paint. Hover or keyboard focus on
the trigger opens it after 400 ms with the endpoint (mono), the MTU and the reason.
Props:
  - path: PathKind; relay: string | null.
  - address: string | null, the endpoint when it is a real host:port.
  - mtu: number | null; reason: string | null.
  - trigger: Snippet<[Record<string, unknown>]>, the badge; spread the props on it.
-->
<script lang="ts">
  import { LinkPreview } from 'bits-ui';
  import type { Snippet } from 'svelte';
  import { copy } from '../copy';
  import { HOVERCARD_DELAY_MS } from '../live/timing';
  import type { PathKind } from '../types.gen';
  import '../ui/overlay.css';
  import { pathText, sentence } from './devices';

  let {
    path,
    relay,
    address,
    mtu,
    reason,
    trigger,
  }: {
    path: PathKind;
    relay: string | null;
    address: string | null;
    mtu: number | null;
    reason: string | null;
    trigger: Snippet<[Record<string, unknown>]>;
  } = $props();
</script>

<LinkPreview.Root openDelay={HOVERCARD_DELAY_MS}>
  <LinkPreview.Trigger>
    {#snippet child({ props })}
      {@render trigger(props)}
    {/snippet}
  </LinkPreview.Trigger>
  <LinkPreview.Portal>
    <LinkPreview.Content
      class="kith-float kith-pop path-card"
      side="bottom"
      align="start"
      sideOffset={8}
    >
      <div class="facts">
        {#if address}<p class="mono">{address}</p>
        {:else if relay && path !== 'direct'}<p>{pathText(path, relay)}</p>{/if}
        {#if mtu !== null}<p>{copy.path.mtu(mtu)}</p>{/if}
        {#if reason}<p class="reason">{sentence(reason)}</p>{/if}
      </div>
    </LinkPreview.Content>
  </LinkPreview.Portal>
</LinkPreview.Root>

<style>
  /* The card carries its own surface so it never depends on another popover being loaded. */
  :global(.path-card) {
    z-index: var(--z-popover);
    max-width: calc(var(--detail) - var(--s-6));
    padding: var(--s-3);
    border-radius: var(--r-3);
    border: var(--hairline) solid var(--line);
    background: var(--surface);
    color: var(--ink-1);
    box-shadow: var(--shadow-pop);
  }
  .facts {
    display: grid;
    gap: var(--s-1);
  }
  .mono {
    font: var(--t-mono);
  }
  .reason {
    color: var(--ink-2);
  }
</style>
