<!--
@component
ExitChip (spec §5): `Exit · vps` with the globe icon, shown only while an exit is in use.
It opens the ExitPicker popover. When the exit is offline it reads, in `--danger`,
`vps offline · traffic blocked` or `vps offline · going direct`, as the daemon reports.
Props:
  - net: string, the network id the picker lists exits for.
-->
<script lang="ts">
  import ExitPicker from '../lib/kith/ExitPicker.svelte';
  import { copy } from '../lib/copy';
  import Icon from '../lib/icons/Icon.svelte';
  import { live } from '../lib/live/store.svelte';
  import Popover from '../lib/ui/Popover.svelte';
  import { exitName } from './net';

  let { net }: { net: string } = $props();

  const exit = $derived(exitName());
  const exitState = $derived(live.status?.exit_state ?? 'active');
  const text = $derived(
    exit === null
      ? ''
      : exitState === 'offline_blocked'
        ? copy.header.exitOfflineBlocked(exit)
        : exitState === 'offline_direct'
          ? copy.header.exitOfflineDirect(exit)
          : copy.header.exitChip(exit),
  );
  let open = $state(false);
</script>

{#if exit !== null}
  <Popover bind:open align="end" label={copy.exits.heading}>
    {#snippet trigger(props)}
      <button type="button" class="chip" class:down={exitState !== 'active'} {...props}>
        <Icon name="globe" />
        <span>{text}</span>
      </button>
    {/snippet}
    <ExitPicker {net} />
  </Popover>
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    color: var(--ink-2);
    white-space: nowrap;
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
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
  @media (pointer: coarse) {
    .chip {
      height: var(--target-touch);
    }
  }
</style>
