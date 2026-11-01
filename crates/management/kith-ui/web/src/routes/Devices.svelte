<!--
@component
Devices (spec §6.3), and the frame of the device detail (§4.3, §4.4, §6.4).
Toolbar: `Filter by name or address` (`/` focuses it), the chips `All 12 · Online 11 ·
Offline 1 · Via relay 2 · Phones 2 · Exits 1`, `Sort: Name ▾` (or `Sorted at 14:02 ·
Re-sort` when frozen by traffic), and `List | Map` on the right; admins also get
`+ Add device` here on mobile. Everything syncs to the URL (`?view= &q= &filter=`).
With a device open (`/n/:net/devices/:dev`) the detail is a 440 px split panel from
1280 px (the list stays interactive and ↑/↓ follow), an overlay drawer from 640 px, and a
full page with a back arrow below that, whose Back restores the list's scroll. Esc closes it.
`?remove=<dev>` opens the RemoveDevice dialog over either.
Props:
  - net: string, the network as the URL names it.
  - query: URLSearchParams.
  - dev?: string | null, the device as the URL names it.
-->
<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { MediaQuery } from 'svelte/reactivity';
  import { copy, DOT } from '../lib/copy';
  import { clock } from '../lib/format';
  import DeviceDetail from '../lib/kith/DeviceDetail.svelte';
  import DeviceList from '../lib/kith/DeviceList.svelte';
  import { deviceSort, listScroll, openedFromList } from '../lib/kith/deviceState.svelte';
  import {
    countDevices,
    deviceItems,
    matchesFilter,
    matchesQuery,
    membershipOf,
    parseFilter,
    resolveDevice,
    resolveNetwork,
    selfPeer,
    type DeviceItem,
  } from '../lib/kith/devices';
  import { actions, live } from '../lib/live/store.svelte';
  import { BANNER_DELAY_MS } from '../lib/live/timing';
  import Map from '../lib/map/Map.svelte';
  import type { DevicesFilter, Range } from '../lib/types';
  import Button from '../lib/ui/Button.svelte';
  import DropdownMenu from '../lib/ui/DropdownMenu.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import { go, href, setQuery } from '../router';
  import RemoveDevice from '../sheets/RemoveDevice.svelte';

  let { net, query, dev = null }: { net: string; query: URLSearchParams; dev?: string | null } =
    $props();

  const wide = new MediaQuery('min-width: 80rem');
  const narrow = new MediaQuery('max-width: 39.999rem');
  const mode = $derived(narrow.current ? 'page' : wide.current ? 'split' : 'drawer');

  const network = $derived(resolveNetwork(live.status, net));
  const netId = $derived(network?.id ?? '');
  const networkName = $derived(network?.name ?? net);
  const admin = $derived(network?.admin ?? false);
  const self = $derived(selfPeer(live.status, membershipOf(live.status, netId)));
  const items = $derived(deviceItems(netId, live.peers[netId], self));

  let exitsAsked = '';
  $effect(() => {
    if (!netId || live.exits[netId] || exitsAsked === netId) return;
    exitsAsked = netId;
    untrack(() => actions.loadExits(netId));
  });
  const exitNodes = $derived(new Set((live.exits[netId] ?? []).map((e) => e.node)));
  const counts = $derived(countDevices(items, exitNodes));

  // §8.5: stale values show after BANNER_DELAY_MS more.
  let staleShown = $state(false);
  $effect(() => {
    if (live.conn !== 'stale' && live.conn !== 'daemon_down') {
      staleShown = false;
      return;
    }
    const timer = setTimeout(() => (staleShown = true), BANNER_DELAY_MS);
    return () => clearTimeout(timer);
  });

  // Toolbar state, from the URL.
  const view = $derived(query.get('view') === 'map' ? 'map' : 'list');
  const filter = $derived(parseFilter(query.get('filter')));
  let search = $state(untrack(() => query.get('q') ?? ''));
  $effect(() => {
    const q = query.get('q') ?? '';
    if (untrack(() => search) !== q) search = q;
  });
  let searchInput: HTMLInputElement | null = $state(null);

  const sort = $derived(deviceSort.get(netId) ?? { by: 'name' as const });
  const sorted: DeviceItem[] = $derived.by(() => {
    if (sort.by === 'name') return items;
    // Frozen order first; devices that arrived since keep their name order after it.
    const rank = new globalThis.Map(sort.order.map((node, i) => [node, i]));
    const pinned = items.filter((i) => i.self);
    const ordered = items
      .filter((i) => !i.self)
      .map((item, i) => ({ item, i }))
      .sort(
        (a, b) =>
          (rank.get(a.item.peer.node) ?? rank.size + a.i) -
          (rank.get(b.item.peer.node) ?? rank.size + b.i),
      )
      .map(({ item }) => item);
    return [...pinned, ...ordered];
  });
  const visible = $derived(
    sorted.filter(
      (item) => matchesFilter(item, filter, exitNodes) && matchesQuery(item, search),
    ),
  );

  function freezeByTraffic(): void {
    const order = items
      .filter((i) => !i.self)
      .map((i) => ({ node: i.peer.node, rx: live.numbers[i.key]?.rx ?? 0, name: i.peer.name }))
      .sort((a, b) => b.rx - a.rx)
      .map((i) => i.node);
    deviceSort.set(netId, { by: 'traffic', order, at: live.now() });
  }

  const chipOptions = $derived([
    { value: 'all', label: copy.devices.chips.all(counts.all) },
    { value: 'online', label: copy.devices.chips.online(counts.online) },
    { value: 'offline', label: copy.devices.chips.offline(counts.offline) },
    { value: 'relayed', label: copy.devices.chips.relayed(counts.relayed) },
    { value: 'phones', label: copy.devices.chips.phones(counts.phones) },
    { value: 'exits', label: copy.devices.chips.exits(counts.exits) },
  ]);
  const viewOptions = [
    { value: 'list', label: copy.devices.list },
    { value: 'map', label: copy.devices.map },
  ];

  // The open device, kept across a rename so the URL can follow its new name.
  let stickyNode: string | null = null;
  const selected: DeviceItem | null = $derived.by(() => {
    if (dev === null) return null;
    const found = resolveDevice(items, dev);
    if (found) return found;
    return items.find((i) => i.peer.node === stickyNode) ?? null;
  });
  $effect(() => {
    if (dev === null) {
      stickyNode = null;
      return;
    }
    if (selected) {
      stickyNode = selected.peer.node;
      if (selected.peer.name !== dev) {
        go(`${deviceHref(selected.peer.name)}${location.search}`, { replace: true });
      }
    }
  });
  const range: Range = $derived.by(() => {
    const r = query.get('range');
    return r === '1m' || r === '15m' ? r : '1h';
  });

  /** The list's query (view, filter, search) without the detail's (range, at, remove). */
  function listSearch(): string {
    // eslint-disable-next-line svelte/prefer-svelte-reactivity -- a throwaway copy
    const next = new URLSearchParams(location.search);
    for (const key of ['range', 'at', 'remove', 'need']) next.delete(key);
    const s = next.toString();
    return s ? `?${s}` : '';
  }

  function deviceHref(name: string): string {
    return href({ name: 'device', net, dev: name });
  }

  function open(item: DeviceItem, options: { replace?: boolean } = {}): void {
    if (mode === 'page') {
      listScroll.set(netId, window.scrollY);
      window.scrollTo(0, 0);
    }
    openedFromList.value = true;
    go(`${deviceHref(item.peer.name)}${listSearch()}`, {
      replace: options.replace ?? dev !== null,
    });
  }

  function openNode(node: string): void {
    const item = items.find((i) => i.peer.node === node);
    if (item) open(item);
  }

  async function close(): Promise<void> {
    const closing = selected?.peer.name ?? dev;
    if (mode === 'page' && openedFromList.value) {
      openedFromList.value = false;
      history.back();
      return;
    }
    openedFromList.value = false;
    go(`${href({ name: 'devices', net })}${listSearch()}`);
    await tick();
    if (closing) {
      document
        .querySelector<HTMLElement>(`[role='row'][data-name='${CSS.escape(closing)}']`)
        ?.focus();
    }
  }

  onMount(() => {
    // The route changed from the list to a device, so this screen is new: keep the focus
    // on the row that opened the split detail, as if the list had never moved.
    if (dev !== null && mode === 'split' && openedFromList.value) {
      const name = dev;
      requestAnimationFrame(() =>
        document
          .querySelector<HTMLElement>(`[role='row'][data-name='${CSS.escape(name)}']`)
          ?.focus({ preventScroll: true }),
      );
    }
    if (dev !== null || !netId) return;
    const y = listScroll.get(netId);
    if (y === undefined) return;
    listScroll.delete(netId);
    requestAnimationFrame(() => window.scrollTo(0, y));
  });

  function editable(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLElement &&
      (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
    );
  }

  function layerOpen(): boolean {
    return (
      document.querySelector(
        "[role='dialog'][data-state='open'], [role='alertdialog'][data-state='open'], [role='menu'][data-state='open']",
      ) !== null
    );
  }

  function keydown(event: KeyboardEvent): void {
    if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.key === '/' && !editable(event.target) && view === 'list') {
      event.preventDefault();
      searchInput?.focus();
      return;
    }
    if (event.key === 'Escape' && dev !== null && !layerOpen()) {
      if (editable(event.target) && event.target === searchInput) return;
      event.preventDefault();
      void close();
    }
  }

  // ?remove=<dev>
  const removing = $derived.by(() => {
    const name = query.get('remove');
    return name === null ? null : resolveDevice(items, name);
  });
</script>

<svelte:window onkeydown={keydown} />

<div
  class="screen"
  class:split={dev !== null && mode === 'split'}
  class:paged={dev !== null && mode === 'page'}
>
  <section class="list" aria-label={copy.deviceView.listLabel(networkName)}>
    <div class="toolbar">
      {#if view === 'list'}
        <div class="search">
          <Input
            bind:ref={searchInput}
            kind="search"
            value={search}
            placeholder={copy.devices.filter}
            aria-label={copy.devices.filterLabel}
            oninput={(event) => {
              search = event.currentTarget.value;
              setQuery({ q: search.trim() === '' ? null : search }, { replace: true });
            }}
          />
        </div>
      {/if}
      <div class="end">
        {#if admin}
          <span class="mobile-add">
            <Button onclick={() => setQuery({ add: 'device' })}>
              {copy.deviceView.addDeviceMobile}
            </Button>
          </span>
        {/if}
        <Segmented
          value={view}
          label={copy.deviceView.viewLabel}
          options={viewOptions}
          onValueChange={(next) => setQuery({ view: next === 'map' ? 'map' : null }, { replace: true })}
        />
      </div>
      {#if view === 'list'}
        <div class="chips">
          <Segmented
            value={filter ?? 'all'}
            label={copy.deviceView.filterChipsLabel}
            options={chipOptions}
            onValueChange={(next) =>
              setQuery({ filter: next === 'all' ? null : (next as DevicesFilter) }, { replace: true })}
          />
        </div>
        <!-- §6.3: `Sort: Name ▾`; frozen by traffic it reads `Sorted at 14:02 · Re-sort`. -->
        <div class="sort">
          <DropdownMenu
            items={[
              { label: copy.devices.sortName, onSelect: () => deviceSort.set(netId, { by: 'name' }) },
              { label: copy.devices.sortTraffic, onSelect: freezeByTraffic },
            ]}
          >
            {#snippet trigger(props)}
              <Button variant="ghost" {...props}>
                {sort.by === 'traffic'
                  ? copy.devices.sortedAt(clock(sort.at))
                  : copy.deviceView.sortBy(copy.devices.sortName)}<span
                  class="caret"
                  aria-hidden="true">{copy.deviceView.caret}</span
                >
              </Button>
            {/snippet}
          </DropdownMenu>
          {#if sort.by === 'traffic'}
            <span class="sep" aria-hidden="true">{DOT.trim()}</span>
            <Button variant="ghost" onclick={freezeByTraffic}>{copy.devices.resort}</Button>
          {/if}
        </div>
      {/if}
    </div>

    {#if items.length <= 1}
      <div class="empty">
        <p>{copy.devices.onlyThis(networkName)}</p>
        {#if admin}
          <div class="empty-actions">
            <Button variant="primary" onclick={() => setQuery({ add: 'device' })}>
              {copy.devices.addDevice}
            </Button>
            <Button onclick={() => setQuery({ add: 'phone' })}>{copy.devices.addPhone}</Button>
          </div>
        {/if}
      </div>
    {:else if view === 'map'}
      <Map net={netId} focused={selected?.peer.node ?? null} onSelect={openNode} />
    {:else if visible.length === 0}
      <p class="none">{copy.devices.noMatch(search.trim())}</p>
    {:else}
      <DeviceList
        net={netId}
        items={visible}
        label={copy.deviceView.listLabel(networkName)}
        selected={selected?.peer.node ?? null}
        followFocus={mode === 'split' && dev !== null}
        stale={staleShown}
        onOpen={(item) => open(item)}
      />
    {/if}
  </section>

  {#if dev !== null}
    {#key selected?.peer.node ?? dev}
      <aside class="detail {mode}" aria-label={copy.deviceView.detailLabel(selected?.peer.name ?? dev)}>
        <DeviceDetail
          net={netId}
          node={selected?.peer.node ?? ''}
          name={dev}
          {mode}
          {range}
          stale={staleShown}
          autofocus={mode !== 'split'}
          onClose={() => void close()}
        />
      </aside>
    {/key}
  {/if}
</div>

{#if removing}
  <RemoveDevice
    bind:open={
      () => removing !== null,
      (next) => {
        if (!next) setQuery({ remove: null });
      }
    }
    net={netId}
    node={removing.peer.node}
    name={removing.peer.name}
    network={networkName}
  />
{/if}

<style>
  .screen {
    --sticky-top: var(--header);
    display: grid;
    gap: var(--s-5);
    max-width: var(--content-max);
    margin: 0 auto;
    padding: var(--s-6) var(--content-pad);
  }
  .list {
    display: grid;
    align-content: start;
    gap: var(--s-4);
    min-width: 0;
  }
  .toolbar {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-areas:
      'search end'
      'chips sort';
    align-items: center;
    gap: var(--s-3) var(--s-4);
  }
  .search {
    grid-area: search;
    max-width: calc(var(--detail) - var(--s-8));
  }
  .end {
    grid-area: end;
    display: flex;
    align-items: center;
    gap: var(--s-3);
    justify-self: end;
  }
  .chips {
    grid-area: chips;
    min-width: 0;
  }
  .chips :global(.track) {
    flex-wrap: wrap;
    height: auto;
    min-height: var(--control);
  }
  .chips :global(.segment) {
    min-height: calc(var(--control) - var(--s-1));
  }
  .sort {
    grid-area: sort;
    display: flex;
    align-items: center;
    gap: var(--s-2);
    justify-self: end;
  }
  .sep {
    color: var(--ink-3);
  }
  .caret {
    margin-left: var(--s-1);
    color: var(--ink-3);
  }
  .mobile-add {
    display: none;
  }
  .empty {
    display: grid;
    justify-items: start;
    gap: var(--s-4);
    padding-block: var(--s-7);
  }
  .empty p,
  .none {
    color: var(--ink-2);
    max-width: var(--measure);
  }
  .none {
    padding-block: var(--s-5);
  }
  .empty-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3);
  }
  .detail {
    background: var(--surface);
  }

  /* ≥1280 px: a 440 px split panel; the list stays interactive. */
  .split {
    max-width: calc(var(--content-max) + var(--detail));
    grid-template-columns: minmax(0, 1fr) var(--detail);
    padding-right: 0;
  }
  .detail.split {
    position: sticky;
    top: var(--header);
    align-self: start;
    min-height: calc(100dvh - var(--header));
    max-height: calc(100dvh - var(--header));
    overflow-y: auto;
    border-left: var(--hairline) solid var(--line);
    margin-block: calc(-1 * var(--s-6));
  }

  /* 640–1279 px: an overlay drawer from the right; the list behind it does not move. */
  .detail.drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: var(--z-drawer);
    width: var(--detail);
    max-width: 100vw;
    overflow-y: auto;
    box-shadow: var(--shadow-pop);
    animation: drawer-in var(--d-sheet) var(--e-sheet);
  }
  @keyframes drawer-in {
    from {
      transform: translateX(100%);
    }
  }

  /* <640 px: the detail is a full page with a back arrow. */
  .detail.page {
    background: var(--bg);
    margin-inline: calc(-1 * var(--gutter-mobile));
  }
  .paged .list {
    display: none;
  }

  @media (max-width: 40rem) {
    .screen {
      --sticky-top: var(--topbar);
      padding: var(--s-4) var(--gutter-mobile);
    }
    .toolbar {
      grid-template-columns: minmax(0, 1fr) auto;
      grid-template-areas:
        'search search'
        'chips chips'
        'sort end';
    }
    .search {
      max-width: none;
    }
    .mobile-add {
      display: inline-flex;
    }
    .end {
      justify-self: end;
    }
    .sort {
      justify-self: start;
    }
  }
</style>
