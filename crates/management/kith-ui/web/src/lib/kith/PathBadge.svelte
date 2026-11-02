<!--
@component
PathBadge (spec §5): text plus glyph, no pill fill. `direct`, `via vps`, `stealth via vps`.
Relayed is not a fault, so no path gets a colour. A change of path waits for 5 s of
stability before the badge follows, so a flap does not flicker. More than 3 changes in a
minute add `flapping` in ink-3.
Hover or keyboard focus opens a card after 400 ms with the endpoint (mono), the MTU and
the reason as a sentence (PathCard, which loads after the first paint; until then the
badge shows alone). On touch it does nothing: the row opens the detail instead.
Props:
  - path: PathKind | null; nothing renders while null.
  - relay?: string | null, the relay's name for `via vps`.
  - flapping?: boolean.
  - endpoint?: string | null, the WireGuard endpoint, for the card.
  - mtu?: number | null, the path MTU when smaller than the tunnel's, for the card.
  - reason?: string | null, the daemon's `path_reason`, for the card.
  - focusable?: boolean (default true). Rows of a grid pass false: the row holds focus,
    and the badge is plain text with no button role.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { copy } from '../copy';
  import Icon from '../icons/Icon.svelte';
  import type { IconName } from '../icons/names';
  import { PATH_HYSTERESIS_MS } from '../live/timing';
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
  /** The daemon sends `relay` as a placeholder; only a real host:port is mono. */
  const address = $derived(endpoint !== null && /[:.]\d/.test(endpoint) ? endpoint : null);
  const hasCard = $derived(
    !coarse && focusable && (address !== null || mtu !== null || reason !== null),
  );
</script>

{#snippet content(path: PathKind)}
  <Icon name={GLYPHS[path]} />
  <span class="text">{pathText(path, shown.relay)}</span>
  {#if flapping}<span class="flap">{copy.path.flapping}</span>{/if}
{/snippet}

{#snippet badge(props: Record<string, unknown>)}
  <span class="badge" {...props} role="button" tabindex="0">
    {@render content(shown.path ?? 'direct')}
  </span>
{/snippet}

{#if hasCard && shown.path}
  {#await import('./PathCard.svelte')}
    {@render badge({})}
  {:then { default: PathCard }}
    <PathCard path={shown.path} relay={shown.relay} {address} {mtu} {reason} trigger={badge} />
  {/await}
{:else if shown.path}
  <span class="badge" title={reason ? sentence(reason) : undefined}>{@render content(shown.path)}</span>
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
</style>
