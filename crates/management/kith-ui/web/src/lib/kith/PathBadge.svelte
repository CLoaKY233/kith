<!--
@component
PathBadge (spec §5): text plus glyph, no pill fill. `direct`, `via vps`, `stealth via vps`.
Relayed is not a fault, so no path gets a colour. A change of path waits for 5 s of
stability before the badge follows, so a flap does not flicker. More than 3 changes in a
minute add `flapping` in ink-3.
Hover or keyboard focus opens a card after 400 ms with the endpoint (mono), the MTU and
the reason as a sentence. On touch it does nothing: the row opens the detail instead.
Props:
  - path: PathKind | null; nothing renders while null.
  - relay?: string | null, the relay's name for `via vps`.
  - flapping?: boolean.
  - endpoint?: string | null, the WireGuard endpoint, for the card.
  - mtu?: number | null, the path MTU when smaller than the tunnel's, for the card.
  - reason?: string | null, the daemon's `path_reason`, for the card.
  - focusable?: boolean (default true). Rows of a grid pass false: the row holds focus.
-->
<script lang="ts">
  import { LinkPreview } from 'bits-ui';
  import { untrack } from 'svelte';
  import { copy } from '../copy';
  import Icon from '../icons/Icon.svelte';
  import type { IconName } from '../icons/names';
  import { HOVERCARD_DELAY_MS, PATH_HYSTERESIS_MS } from '../live/timing';
  import type { PathKind } from '../types.gen';
  import { pathText, sentence } from './devices';

  let {
    path,
    relay = null,
    flapping = false,
    endpoint = null,
    mtu = null,
    reason = null,
    focusable = true,
  }: {
    path: PathKind | null;
    relay?: string | null;
    flapping?: boolean;
    endpoint?: string | null;
    mtu?: number | null;
    reason?: string | null;
    focusable?: boolean;
  } = $props();

  let shown = $state(untrack(() => ({ path, relay })));

  $effect(() => {
    const next = { path, relay };
    if (next.path === shown.path && next.relay === shown.relay) return;
    // The first path a device gets shows at once, and so does a relay's name; only a
    // change of path kind waits.
    if (shown.path === null || next.path === shown.path) {
      shown = next;
      return;
    }
    const timer = setTimeout(() => (shown = next), PATH_HYSTERESIS_MS);
    return () => clearTimeout(timer);
  });

  const GLYPHS: Record<PathKind, IconName> = {
    direct: 'path-direct',
    relay: 'path-relayed',
    stealth: 'path-stealth',
  };

  const coarse =
    typeof window !== 'undefined' && window.matchMedia('(pointer: coarse)').matches;
  const hasCard = $derived(!coarse && (endpoint !== null || mtu !== null || reason !== null));
</script>

{#snippet content(path: PathKind)}
  <Icon name={GLYPHS[path]} />
  <span class="text">{pathText(path, shown.relay)}</span>
  {#if flapping}<span class="flap">{copy.path.flapping}</span>{/if}
{/snippet}

{#if hasCard && shown.path}
  <LinkPreview.Root openDelay={HOVERCARD_DELAY_MS}>
    <LinkPreview.Trigger>
      {#snippet child({ props })}
        <span class="badge" {...props} role="button" tabindex={focusable ? 0 : -1}>
          {@render content(shown.path ?? 'direct')}
        </span>
      {/snippet}
    </LinkPreview.Trigger>
    <LinkPreview.Portal>
      <LinkPreview.Content class="kith-pop path-card" side="bottom" align="start" sideOffset={8}>
        <div class="facts">
          {#if endpoint}<p class="mono">{endpoint}</p>{/if}
          {#if mtu !== null}<p>{copy.path.mtu(mtu)}</p>{/if}
          {#if reason}<p class="reason">{sentence(reason)}</p>{/if}
        </div>
      </LinkPreview.Content>
    </LinkPreview.Portal>
  </LinkPreview.Root>
{:else if shown.path}
  <span class="badge">{@render content(shown.path)}</span>
{/if}

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    max-width: 100%;
    color: var(--ink-2);
    white-space: nowrap;
    border-radius: var(--r-1);
  }
  /* The interactive badge reaches the 24 px fine-pointer target (§11.4). */
  .badge[role='button'] {
    min-height: var(--target-fine);
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .flap {
    flex: none;
    font: var(--t-small);
    color: var(--ink-3);
  }
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
  :global(.path-card[data-state='open']) {
    animation: path-card-in var(--d-pop-in) var(--e-pop);
  }
  @keyframes -global-path-card-in {
    from {
      opacity: 0;
    }
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
