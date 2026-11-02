<!--
@component
Activity (spec §6.5, §7.12). The toolbar (kind, device, search) lives in the URL as
`?kind=&device=&q=`. The feed is a virtualised `role="log"` with `aria-live=off`, newest
first, with day and gap dividers, grouped rows and the cap row. Once scrolled more than
48 px from the top it stops taking new entries and shows `12 new ↑`; the pill, or
scrolling back up (Home), shows them.
Props:
  - net: string, the network as the route names it.
  - query: URLSearchParams, the route's query.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { setQuery } from '../router';
  import { copy, copyScreens } from '../lib/copy';
  import {
    ACTIVITY_KINDS,
    buildFeed,
    matches,
    startOfDay,
    type FeedRow,
    type Filter,
  } from '../lib/kith/activity';
  import EventRow from '../lib/kith/EventRow.svelte';
  import { liveContext } from '../lib/kith/liveContext';
  import { devicesByName, findNetwork, membershipOf } from '../lib/kith/network';
  import VirtualList from '../lib/kith/VirtualList.svelte';
  import { live, notifications } from '../lib/live/store.svelte';
  import { SORT_FREEZE_MS, TINT_MS } from '../lib/live/timing';
  import { wasHiddenAt } from '../lib/prefs.svelte';
  import { readPx } from '../lib/tokens';
  import type { Entry } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';

  let { net, query }: { net: string; query: URLSearchParams } = $props();

  const network = $derived(findNetwork(live.status, net));
  const kind = $derived(ACTIVITY_KINDS.find((k) => k === query.get('kind')) ?? null);
  const device = $derived(query.get('device') || null);
  const search = $derived(query.get('q') ?? '');
  const filter: Filter = $derived({
    network: network?.id ?? null,
    kind,
    device,
    query: search,
  });

  const ctx = liveContext;

  const devices = $derived(network ? devicesByName(live.peers, network.id) : []);
  const self = $derived(network ? membershipOf(live.status, network.id) : null);
  /** The device filter as the select shows it: by name, even when the URL holds an id. */
  const deviceName = $derived(devices.find((p) => p.node === device)?.name ?? device ?? '');
  const deviceKnown = $derived(
    deviceName === '' || deviceName === self?.name || devices.some((p) => p.name === deviceName),
  );

  const kindOptions = [
    { value: 'all', label: copy.activity.kinds.all },
    ...ACTIVITY_KINDS.map((k) => ({ value: k, label: copy.activity.kinds[k] })),
  ];

  function pickKind(value: string): void {
    setQuery({ kind: value === 'all' ? null : value }, { replace: true });
  }

  // §8.7 and §3.5: the feed stops taking new entries once scrolled more than 48 px from
  // the top, and while the pointer or focus is in it, so no row moves under either.
  let pausePx = 0;
  let away = false;
  let held = false;
  let release: ReturnType<typeof setTimeout> | null = null;
  let cutoff = $state<number | null>(null);
  let list: { toTop(): void } | null = $state(null);
  const rowPx = readPx('row');
  const latestSeq = $derived(live.journal[live.journal.length - 1]?.seq ?? 0);

  function settle(): void {
    if (away || held) cutoff ??= latestSeq;
    else cutoff = null;
  }

  function scrolled(offset: number): void {
    away = offset > pausePx;
    settle();
  }

  function hold(): void {
    if (release !== null) clearTimeout(release);
    release = null;
    held = true;
    settle();
  }

  function letGo(event: FocusEvent | PointerEvent): void {
    const feed = event.currentTarget as HTMLElement;
    if (event instanceof FocusEvent && feed.contains(event.relatedTarget as Node | null)) return;
    if (release !== null) clearTimeout(release);
    release = setTimeout(() => {
      release = null;
      held = feed.matches(':hover') || feed.contains(document.activeElement);
      settle();
    }, SORT_FREEZE_MS);
  }

  const shown = $derived(
    cutoff === null ? live.journal : live.journal.filter((e) => e.seq <= (cutoff ?? 0)),
  );
  const newer = $derived(
    cutoff === null
      ? 0
      : live.journal.filter((e) => e.seq > (cutoff ?? 0) && matches(e, filter, ctx)).length,
  );

  /** Day labels follow the server clock; this changes once a day. */
  const today = $derived(startOfDay(live.lastTickMs || live.now()));
  const gaps = $derived(live.gaps.map((g) => ({ from: g.from, to: g.to, at_ms: g.toMs ?? g.fromMs })));
  const rows = $derived(buildFeed(live.journal, shown, filter, ctx, gaps, today));

  // Rows expanded by the person, kept here because virtualised rows unmount.
  const open = new SvelteSet<string>();
  const seqAtOpen = live.seq;

  function isFresh(entry: Entry): boolean {
    return entry.seq > seqAtOpen && live.now() - entry.at_ms < TINT_MS;
  }

  function showNewer(): void {
    list?.toTop();
    away = false;
    cutoff = held ? latestSeq : null;
  }

  // §7.12: offer notifications after something that would have notified arrived while
  // the tab was hidden.
  let askDismissed = $state(false);
  function recentlyUsed(network: string, node: string): boolean {
    const columns = live.traffic.get(`${network}/${node}`);
    if (!columns || columns.length < 2) return false;
    return (columns.rx[columns.length - 1] ?? 0) > (columns.rx[0] ?? 0);
  }
  const wouldHaveNotified = $derived(
    live.journal.some((e) => {
      if (!wasHiddenAt(e.at_ms - live.serverOffset)) return false;
      if (e.kind === 'join_requested') {
        return membershipOf(live.status, e.network)?.role === 'admin';
      }
      return e.kind === 'peer_offline' && recentlyUsed(e.network, e.node);
    }),
  );
  const ask = $derived(
    !askDismissed && notifications.offerable && (notifications.missed || wouldHaveNotified),
  );

  async function turnOn(): Promise<void> {
    await notifications.turnOn();
    askDismissed = true;
  }

  function notNow(): void {
    notifications.notNow();
    askDismissed = true;
  }

  const emptyText = $derived(
    search.trim() !== ''
      ? copyScreens.activity.noMatch(search.trim())
      : kind !== null || device !== null
        ? copyScreens.activity.noneOfKind
        : copy.activity.empty,
  );
  const hasEntries = $derived(rows.some((r) => r.type === 'entry' || r.type === 'group'));

  onMount(() => {
    pausePx = readPx('journal-pause');
  });

  const rowKey = (row: FeedRow): string => row.key;
</script>

<section class="screen">
  <h1 class="t-title">{copy.nav.activity}</h1>

  {#if ask}
    <div class="ask">
      <h2 class="t-heading">{copy.activity.notifyCardTitle}</h2>
      <p class="ask-body">{copy.activity.notifyCardBody}</p>
      <div class="ask-actions">
        <Button variant="primary" onclick={turnOn}>{copy.activity.notifyOn}</Button>
        <Button variant="ghost" onclick={notNow}>{copy.activity.notifyNotNow}</Button>
      </div>
    </div>
  {/if}

  <div class="toolbar">
    <div class="kinds-wide">
      <Segmented
        value={kind ?? 'all'}
        options={kindOptions}
        label={copyScreens.activity.kind}
        onValueChange={pickKind}
      />
    </div>
    <label class="select kinds-narrow">
      <span class="sr-only">{copyScreens.activity.kind}</span>
      <select value={kind ?? 'all'} onchange={(e) => pickKind(e.currentTarget.value)}>
        {#each kindOptions as option (option.value)}
          <option value={option.value}>{option.label}</option>
        {/each}
      </select>
    </label>
    <label class="select">
      <span class="sr-only">{copy.activity.device}</span>
      <select
        value={deviceName}
        onchange={(e) => setQuery({ device: e.currentTarget.value || null }, { replace: true })}
      >
        <option value="">{copy.activity.anyDevice}</option>
        {#if self}<option value={self.name}>{self.name}</option>{/if}
        {#each devices as peer (peer.node)}
          <option value={peer.name}>{peer.name}</option>
        {/each}
        {#if !deviceKnown}<option value={deviceName}>{deviceName}</option>{/if}
      </select>
    </label>
    <div class="search">
      <Input
        kind="search"
        value={search}
        placeholder={copy.activity.search}
        aria-label={copy.activity.search}
        data-filter
        oninput={(e: Event & { currentTarget: HTMLInputElement }) =>
          setQuery({ q: e.currentTarget.value || null }, { replace: true })}
      />
    </div>
  </div>

  <div
    class="feed"
    role="presentation"
    onpointerenter={hold}
    onpointerleave={letGo}
    onfocusin={hold}
    onfocusout={letGo}
  >
    {#if newer > 0}
      <div class="pill-slot">
        <button
          type="button"
          class="pill"
          aria-label={copyScreens.activity.newerLabel(newer)}
          onclick={showNewer}
        >
          {copy.activity.newer(newer)}
        </button>
      </div>
    {/if}
    <div role="log" aria-live="off" aria-label={copyScreens.activity.feed}>
      {#if hasEntries}
        <VirtualList bind:this={list} items={rows} key={rowKey} estimate={rowPx} onScroll={scrolled}>
          {#snippet row(item: FeedRow)}
            {#if item.type === 'day'}
              <h2 class="divider">{item.label}</h2>
            {:else if item.type === 'gap'}
              <p class="gap">{item.text}</p>
            {:else if item.type === 'cap'}
              <p class="cap">{item.text}</p>
            {:else if item.type === 'group'}
              {@const newest = item.entries[0]}
              {#if newest}
                <EventRow
                  entry={newest}
                  group={item.entries}
                  fresh={isFresh(newest)}
                  expanded={open.has(item.key)}
                  onToggle={(on) => (on ? open.add(item.key) : open.delete(item.key))}
                />
              {/if}
            {:else}
              <EventRow
                entry={item.entry}
                fresh={isFresh(item.entry)}
                expanded={open.has(item.key)}
                onToggle={(on) => (on ? open.add(item.key) : open.delete(item.key))}
              />
            {/if}
          {/snippet}
        </VirtualList>
      {:else}
        {#each rows as item (item.key)}
          {#if item.type === 'gap'}<p class="gap">{item.text}</p>{/if}
        {/each}
        <p class="empty">{emptyText}</p>
      {/if}
    </div>
  </div>
</section>

<style>
  .screen {
    display: grid;
    gap: var(--s-5);
    max-width: var(--content-max);
    margin: 0 auto;
    padding: var(--s-6) var(--content-pad);
  }

  .ask {
    display: grid;
    gap: var(--s-2);
    padding: var(--s-4);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface);
  }
  .ask-body {
    max-width: var(--measure);
    color: var(--ink-2);
  }
  .ask-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3);
    padding-top: var(--s-2);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
  }
  .select.kinds-narrow {
    display: none;
  }
  .search {
    flex: 1 1 calc(var(--s-8) * 4);
    max-width: calc(var(--s-8) * 5);
  }
  .select {
    position: relative;
    display: inline-flex;
  }
  .select select {
    height: var(--control);
    max-width: 100%;
    padding: 0 var(--s-6) 0 var(--s-3);
    border: var(--hairline) solid var(--line-strong);
    border-radius: var(--r-2);
    background: var(--sunken);
    color: var(--ink-1);
    font: var(--t-input);
    cursor: pointer;
  }
  .select select:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }

  .feed {
    position: relative;
  }
  .pill-slot {
    position: sticky;
    top: calc(var(--header) + var(--s-2));
    z-index: var(--z-header);
    display: flex;
    justify-content: center;
    height: 0;
  }
  .pill {
    height: var(--control);
    padding: 0 var(--s-3);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-full);
    background: var(--surface);
    box-shadow: var(--shadow-pop);
    color: var(--ink-1);
    font: var(--t-small);
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .pill:hover {
    background: var(--sunken);
  }
  .pill:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }

  .divider {
    padding: var(--s-5) var(--s-3) var(--s-2);
    color: var(--ink-3);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    font-variant-caps: all-small-caps;
  }
  .gap,
  .cap {
    margin: var(--s-2) 0;
    padding: var(--s-3);
    border-radius: var(--r-2);
    background: var(--sunken);
    color: var(--ink-3);
    font: var(--t-small);
  }
  .empty {
    padding: var(--s-5) var(--s-3);
    color: var(--ink-2);
    max-width: var(--measure);
  }

  @media (max-width: 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
    .kinds-wide {
      display: none;
    }
    .select.kinds-narrow {
      display: inline-flex;
    }
    .toolbar .select {
      flex: 1 1 0;
      min-width: 0;
    }
    .toolbar .select select {
      width: 100%;
    }
    .search {
      flex-basis: 100%;
      max-width: none;
    }
  }
</style>
