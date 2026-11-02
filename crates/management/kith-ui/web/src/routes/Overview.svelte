<!--
@component
Overview (spec §6.2): the 8-second answer. Top to bottom, 48 px apart:
  1. Headline.
  2. Needs you: up to 3 cards, then `View all (5)` (the header popover). Absent when empty.
  3. Devices: up to 8 rows, this device first, then by name, then `All 12 devices →`.
  4. Recent: the last 5 changes and joins, then `All activity →`.
At ≥1600 px Recent becomes a 360 px right column with the whole feed. Phones keep the order.
Props:
  - net: string, the network id.
-->
<script lang="ts">
  import { MediaQuery } from 'svelte/reactivity';
  import { layers } from '../frame/layers.svelte';
  import { peers, selfPeer } from '../frame/net';
  import { needs } from '../frame/needs.svelte';
  import { copy } from '../lib/copy';
  import DeviceRow from '../lib/kith/DeviceRow.svelte';
  import EventRow from '../lib/kith/EventRow.svelte';
  import Headline from '../lib/kith/Headline.svelte';
  import NeedsYouCard from '../lib/kith/NeedsYouCard.svelte';
  import { live } from '../lib/live/store.svelte';
  import type { Entry } from '../lib/types.gen';
  import { follow, go, href, netHref } from '../router';

  let { net }: { net: string } = $props();

  /** §6.2 limits. */
  const NEEDS_SHOWN = 3;
  const DEVICES_SHOWN = 8;
  const RECENT_SHOWN = 5;
  /** The wide column's feed; Activity holds the rest. */
  const FEED_SHOWN = 100;
  /** Entries Recent counts as Changes or Joins (§6.5 kinds). */
  const RECENT_KINDS = new Set<Entry['kind']>([
    'device_joined',
    'device_removed',
    'exit_changed',
    'join_requested',
  ]);
  /** Log levels the feed always shows (§6.5). */
  const LOUD = new Set(['warn', 'error']);

  const wide = new MediaQuery('min-width: 100rem');

  const list = $derived(needs.forNet(net));
  const shownNeeds = $derived(list.slice(0, NEEDS_SHOWN));

  const others = $derived(peers(net).sort((a, b) => a.name.localeCompare(b.name)));
  const me = $derived(selfPeer(net));
  const total = $derived(others.length + 1);
  const rows = $derived(others.slice(0, me ? DEVICES_SHOWN - 1 : DEVICES_SHOWN));

  function inNet(entry: Entry): boolean {
    return !('network' in entry) || entry.network === net;
  }

  const recent = $derived.by(() => {
    const out: Entry[] = [];
    const journal = live.journal;
    const wanted = wide.current ? FEED_SHOWN : RECENT_SHOWN;
    for (let i = journal.length - 1; i >= 0 && out.length < wanted; i -= 1) {
      const entry = journal[i];
      if (!entry || !inNet(entry)) continue;
      const keep = wide.current
        ? entry.kind !== 'log' || LOUD.has(entry.level)
        : RECENT_KINDS.has(entry.kind);
      if (keep) out.push(entry);
    }
    return out;
  });

  const devicesHref = $derived(netHref(net, 'devices'));
  const activityHref = $derived(netHref(net, 'activity'));

  function open(node: string): void {
    go(href({ name: 'device', net, dev: node }));
  }

  /** Roving focus for the rows (§5 DeviceRow): one tab stop, ↑/↓ move between rows. */
  function roving(grid: HTMLElement): () => void {
    const rowsOf = (): HTMLElement[] => [...grid.querySelectorAll<HTMLElement>('[role="row"]')];
    const settle = (): void => {
      const all = rowsOf();
      if (!all.some((r) => r.tabIndex === 0)) all[0]?.setAttribute('tabindex', '0');
    };
    const keys = (event: KeyboardEvent): void => {
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
      const all = rowsOf();
      const at = all.findIndex((r) => r.contains(document.activeElement));
      const next = all[event.key === 'ArrowDown' ? Math.min(all.length - 1, at + 1) : Math.max(0, at - 1)];
      if (!next || at === -1) return;
      event.preventDefault();
      for (const r of all) r.setAttribute('tabindex', r === next ? '0' : '-1');
      next.focus();
    };
    settle();
    const observer = new MutationObserver(settle);
    observer.observe(grid, { childList: true, subtree: true });
    grid.addEventListener('keydown', keys);
    return () => {
      observer.disconnect();
      grid.removeEventListener('keydown', keys);
    };
  }
</script>

<div class="overview" class:wide={wide.current}>
  <div class="main">
    <Headline {net} />

    {#if shownNeeds.length > 0}
      <section class="section" aria-labelledby="needs-title">
        <h2 id="needs-title" class="heading">{copy.overview.needsYou}</h2>
        <div class="cards">
          {#each shownNeeds as need (need.id)}
            <NeedsYouCard {need} primary={shownNeeds.length === 1} />
          {/each}
        </div>
        {#if list.length > NEEDS_SHOWN}
          <div>
            <button type="button" class="more" onclick={() => (layers.needs = true)}>
              {copy.overview.viewAll(list.length)}
            </button>
          </div>
        {/if}
      </section>
    {/if}

    <section class="section" aria-labelledby="devices-title">
      <h2 id="devices-title" class="heading">{copy.overview.devices}</h2>
      <div class="rows" role="grid" aria-labelledby="devices-title" {@attach roving}>
        {#if me}
          <DeviceRow {net} peer={me} self onOpen={() => open(me.node)} />
        {/if}
        {#each rows as peer (peer.node)}
          <DeviceRow {net} {peer} onOpen={() => open(peer.node)} />
        {/each}
      </div>
      <div>
        <a class="more" href={devicesHref} onclick={(e) => follow(e, devicesHref)}>
          {copy.overview.allDevices(total)}
        </a>
      </div>
    </section>

    {#if !wide.current}
      <section class="section" aria-labelledby="recent-title">
        <h2 id="recent-title" class="heading">{copy.overview.recent}</h2>
        {#if recent.length > 0}
          <div class="feed">
            {#each recent as entry (entry.seq)}<EventRow {entry} />{/each}
          </div>
        {/if}
        <div>
          <a class="more" href={activityHref} onclick={(e) => follow(e, activityHref)}>
            {copy.overview.allActivity}
          </a>
        </div>
      </section>
    {/if}
  </div>

  {#if wide.current}
    <aside class="column" aria-labelledby="recent-title">
      <h2 id="recent-title" class="heading">{copy.overview.recent}</h2>
      <div class="feed">
        {#each recent as entry (entry.seq)}<EventRow {entry} />{/each}
      </div>
      <div>
        <a class="more" href={activityHref} onclick={(e) => follow(e, activityHref)}>
          {copy.overview.allActivity}
        </a>
      </div>
    </aside>
  {/if}
</div>

<style>
  .overview {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-7);
    max-width: var(--content-max);
    padding: var(--s-6) var(--content-pad) var(--s-7);
  }
  .overview.wide {
    grid-template-columns: minmax(0, 1fr) var(--recent-column);
    max-width: calc(var(--content-max) + var(--recent-column) + var(--s-7));
  }
  .main {
    display: grid;
    align-content: start;
    gap: var(--s-7);
    min-width: 0;
  }
  .section,
  .column {
    display: grid;
    align-content: start;
    gap: var(--s-4);
    min-width: 0;
  }
  .column {
    position: sticky;
    top: calc(var(--header) + var(--s-6));
    max-height: calc(100dvh - var(--header) - var(--s-7));
    overflow-y: auto;
  }
  .heading {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
  }
  .cards {
    display: grid;
    gap: var(--s-3);
  }
  .rows,
  .feed {
    display: grid;
  }
  .more {
    display: inline-flex;
    align-items: center;
    justify-self: start;
    min-height: var(--target-fine);
    color: var(--ink-2);
    border-radius: var(--r-1);
    cursor: pointer;
    transition: color var(--d-hover) var(--e-linear);
  }
  .more:hover {
    color: var(--ink-1);
    text-decoration: underline;
    text-decoration-thickness: var(--hairline);
    text-underline-offset: var(--s-1);
  }
  @media (pointer: coarse) {
    .more {
      min-height: var(--target-touch);
    }
  }
  @media (max-width: 40rem) {
    .overview {
      padding: var(--s-4) var(--gutter-mobile) var(--s-7);
    }
  }
</style>
