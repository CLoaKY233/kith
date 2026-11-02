<!--
@component
ExitChip (spec §5): `Exit · vps` with the globe icon, shown only while an exit is in use.
It opens the ExitPicker popover (also from ⌘K `Use exit…`, through `layers.exit`). When
the exit is offline it reads, in `--danger`, `vps offline · traffic blocked` or
`vps offline · going direct`, as the daemon reports; compact (the phone top bar) says only
`vps offline` and leaves the rest to the popover. It takes the 4 s change tint when the
exit changes (§8.4). Picking `No exit` keeps the popover open until it is closed, so the
result shows; focus then goes to the content.
Props:
  - net: string, the network id the picker lists exits for.
  - compact?: boolean, the short offline label.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { copy } from '../lib/copy';
  import Icon from '../lib/icons/Icon.svelte';
  import { tintKey } from '../lib/live/keys';
  import { live } from '../lib/live/store.svelte';
  import Popover from '../lib/ui/Popover.svelte';
  import { layers } from './layers.svelte';
  import { exitName } from './net';

  let { net, compact = false }: { net: string; compact?: boolean } = $props();

  const exit = $derived(exitName());
  const exitState = $derived(live.status?.exit_state ?? 'active');
  const text = $derived(
    exit === null
      ? ''
      : exitState === 'offline_blocked'
        ? compact
          ? copy.header.exitOffline(exit)
          : copy.header.exitOfflineBlocked(exit)
        : exitState === 'offline_direct'
          ? compact
            ? copy.header.exitOffline(exit)
            : copy.header.exitOfflineDirect(exit)
          : copy.header.exitChip(exit),
  );
  // Restart the tint on each change without remounting the popover's trigger.
  const changedAt = $derived(live.changed.get(tintKey.exit) ?? null);
  let tint = $state(false);
  $effect(() => {
    if (changedAt === null) {
      tint = false;
      return;
    }
    tint = false;
    const frame = requestAnimationFrame(() => (tint = true));
    return () => cancelAnimationFrame(frame);
  });

  // The chip is gone once the exit is; the popover's own focus return has nowhere to go.
  let wasOpen = false;
  $effect(() => {
    if (layers.exit) {
      wasOpen = true;
      return;
    }
    if (!wasOpen) return;
    wasOpen = false;
    if (exit !== null) return;
    void tick().then(() => {
      if (document.activeElement === document.body) document.getElementById('content')?.focus();
    });
  });
</script>

{#if exit !== null || layers.exit}
  <Popover bind:open={layers.exit} align="end" label={copy.exits.heading}>
    {#snippet trigger(props)}
      <button
        type="button"
        class="chip"
        class:down={exitState !== 'active'}
        class:tint
        class:gone={exit === null}
        {...props}
      >
        <Icon name="globe" />
        <span class="label">{text}</span>
      </button>
    {/snippet}
    {#await import('../lib/kith/ExitPicker.svelte') then { default: ExitPicker }}
      <ExitPicker {net} />
    {/await}
  </Popover>
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    max-width: 100%;
    flex: 0 1 auto;
    height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    color: var(--ink-2);
    white-space: nowrap;
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .chip.gone {
    visibility: hidden;
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip:hover,
  .chip[aria-expanded='true'] {
    background: var(--sunken);
    color: var(--ink-1);
  }
  .chip.down,
  .chip.down:hover {
    color: var(--danger);
  }
  .tint {
    animation: chip-tint var(--d-tint) var(--e-linear);
  }
  @keyframes chip-tint {
    from {
      background-color: var(--changed);
    }
  }
  @media (pointer: coarse) {
    .chip {
      height: var(--target-touch);
    }
  }
</style>
