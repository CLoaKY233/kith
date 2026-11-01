<!--
@component
Needs-you sheets (spec §4.1 `?need=<id>`, §7.12): one NeedsYouCard in a sheet, opened from
a notification, the headline's fix link or a pasted link; Back closes it. It closes by
itself when the card is done or the need is gone. On phones, where the header has no
popover, `View all` opens the whole list here too.
Props:
  - net: string, the network id.
  - id: string | null, the `?need=` value.
  - list?: boolean, show every need (phones' `View all`).
-->
<script lang="ts">
  import { MediaQuery } from 'svelte/reactivity';
  import NeedsYouCard from '../lib/kith/NeedsYouCard.svelte';
  import { copy } from '../lib/copy';
  import Sheet from '../lib/ui/Sheet.svelte';
  import { closeOverlay } from '../router';
  import { layers } from './layers.svelte';
  import { needs } from './needs.svelte';

  let { net, id, list = false }: { net: string; id: string | null; list?: boolean } = $props();

  const need = $derived(id === null ? null : needs.find(id));

  $effect(() => {
    if (id !== null && need === null) closeOverlay('need');
  });

  const all = $derived(needs.forNet(net));
  const phone = new MediaQuery('not all and (min-width: 40rem)');
</script>

{#if list}
  {#if phone.current}
  <Sheet bind:open={layers.needs} title={copy.header.needsYouTitle} closeLabel={copy.action.close}>
    {#each all as item (item.id)}
      <NeedsYouCard need={item} primary={all.length === 1} />
    {/each}
  </Sheet>
  {/if}
{:else}
  <Sheet
    bind:open={() => need !== null, (open) => !open && closeOverlay('need')}
    title={copy.header.needsYouTitle}
    closeLabel={copy.action.close}
  >
    {#if need}
      <NeedsYouCard {need} onDone={() => closeOverlay('need')} />
    {/if}
  </Sheet>
{/if}
