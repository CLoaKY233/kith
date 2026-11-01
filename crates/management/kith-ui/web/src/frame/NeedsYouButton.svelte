<!--
@component
NeedsYouButton (spec §4.2): an `--attn` dot plus the count, shown only when something
needs this person. It opens a popover of NeedsYouCards; Overview's `View all (5)` opens
the same one through `layers.needs`.
Props:
  - net: string, the network id.
-->
<script lang="ts">
  import NeedsYouCard from '../lib/kith/NeedsYouCard.svelte';
  import { copy } from '../lib/copy';
  import Popover from '../lib/ui/Popover.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';
  import { layers } from './layers.svelte';
  import { needs } from './needs.svelte';

  let { net }: { net: string } = $props();

  const count = $derived(needs.count(net));
  const list = $derived(needs.forNet(net));
</script>

{#if count > 0 || layers.needs}
  <Popover bind:open={layers.needs} align="end" width="card" label={copy.header.needsYouTitle}>
    {#snippet trigger(props)}
      <button type="button" class="needs" aria-label={copy.header.needsYou(count)} {...props}>
        <StatusDot state="needs_you" />
        <span>{count}</span>
      </button>
    {/snippet}
    <div class="list">
      <h2 class="heading">{copy.header.needsYouTitle}</h2>
      {#each list as need (need.id)}
        <NeedsYouCard {need} layered primary={need.id === list.find((n) => n.kind === 'join')?.id} />
      {/each}
    </div>
  </Popover>
{/if}

<style>
  .needs {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    color: var(--attn-text);
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .needs:hover,
  .needs[aria-expanded='true'] {
    background: var(--sunken);
  }
  .list {
    display: grid;
    gap: var(--s-3);
    max-height: 70vh;
    overflow-y: auto;
  }
  .heading {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
  }
  @media (pointer: coarse) {
    .needs {
      height: var(--target-touch);
    }
  }
</style>
