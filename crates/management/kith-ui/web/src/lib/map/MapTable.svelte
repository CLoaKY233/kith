<!--
@component
MapTable (spec §9.2): the map as a table, from · to · path · latency, one row per reported
link and one for each device nobody reports. Visually hidden beside the map for screen
readers; the Grid view (above 60 devices) shows it.
Props:
  - net: string, the network id (a network name also resolves).
  - visible?: boolean (default false: visually hidden).
  - onSelect?: (node: string) => void. When set and visible, each `to` name opens that
    device's detail.
-->
<script lang="ts">
  import { copy } from '../copy';
  import { live } from '../live/store.svelte';
  import { BANNER_DELAY_MS } from '../live/timing';
  import { buildModel, LAST_KNOWN, resolveNetwork, tableRows } from './model';

  let {
    net,
    visible = false,
    onSelect,
  }: { net: string; visible?: boolean; onSelect?: (node: string) => void } = $props();

  let stale = $state(false);
  $effect(() => {
    if (!LAST_KNOWN.has(live.conn)) {
      stale = false;
      return;
    }
    const timer = setTimeout(() => (stale = true), BANNER_DELAY_MS);
    return () => clearTimeout(timer);
  });

  const netId = $derived(resolveNetwork(live.status, net));
  const model = $derived(
    netId ? buildModel(live.status, live.peers[netId], live.links[netId], netId) : null,
  );
  const rows = $derived(model ? tableRows(model, stale) : []);
</script>

<table class="map-table" class:sr-only={!visible} aria-label={copy.devices.map}>
  <thead>
    <tr>
      <th scope="col">{copy.map.from}</th>
      <th scope="col">{copy.map.to}</th>
      <th scope="col">{copy.map.path}</th>
      <th scope="col" class="num">{copy.map.latency}</th>
    </tr>
  </thead>
  <tbody>
    {#each rows as row (row.key)}
      <tr>
        <td>{row.from}</td>
        <td>
          {#if visible && onSelect}
            <button type="button" class="open" onclick={() => onSelect(row.toId)}>{row.to}</button>
          {:else}
            {row.to}
          {/if}
        </td>
        <td>{row.path}</td>
        <td class="num">{row.latency}</td>
      </tr>
    {/each}
  </tbody>
</table>

<style>
  .map-table {
    width: 100%;
    border-collapse: collapse;
    font: var(--t-body);
    letter-spacing: var(--t-body-tracking);
    color: var(--ink-1);
  }
  th {
    position: sticky;
    top: 0;
    z-index: var(--z-header);
    height: var(--row);
    padding: 0 var(--s-3);
    border-bottom: var(--hairline) solid var(--line);
    background: var(--bg);
    color: var(--ink-3);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    font-variant-caps: all-small-caps;
    text-align: left;
  }
  td {
    height: var(--row);
    padding: 0 var(--s-3);
    border-bottom: var(--hairline) solid var(--line);
    overflow-wrap: anywhere;
  }
  .num {
    text-align: right;
    white-space: nowrap;
  }
  .open {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    text-decoration: underline;
    text-decoration-color: var(--line-strong);
    text-underline-offset: var(--s-1);
    transition: text-decoration-color var(--d-hover) var(--e-linear);
  }
  .open:hover {
    text-decoration-color: var(--ink-1);
  }
  .open:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
    border-radius: var(--r-1);
  }
  @media (max-width: 40rem) {
    th,
    td {
      padding: 0 var(--s-2);
    }
  }
</style>
