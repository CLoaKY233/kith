<!--
@component
DeviceList (spec §5 DeviceRow, §6.3, §8.7): the `role="grid"` of DeviceRows with a sticky
column header in micro small caps. It is the `devices` size container whose width picks
the row columns. Overview reuses it for its first 8 rows.
Keyboard: ↑/↓ (and Home/End) move between rows with a roving tab stop, Enter opens the
detail, `.` opens the row's ⋯ menu, typing jumps to the next name that starts with what
was typed.
Stability: the order never changes while the pointer is over the list or focus is in it,
nor for 1 s after it leaves; new devices wait at the end. A removed device stays in place
reading `Removed` for 1.5 s, then collapses over 160 ms; new rows grow in the same way.
Props:
  - net: string, network id.
  - items: readonly DeviceItem[], in the order to show (this device first).
  - label: string, the grid's accessible name.
  - selected?: string | null, the node whose detail is open.
  - followFocus?: boolean, ↑/↓ also open the focused device (the split view, §6.4).
  - stale?: boolean, values are last known (§8.5).
  - header?: boolean (default true), shows the column header.
  - onOpen: (item: DeviceItem) => void.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { TransitionConfig } from 'svelte/transition';
  import { SvelteMap } from 'svelte/reactivity';
  import { copy } from '../copy';
  import { live } from '../live/store.svelte';
  import {
    RELATIVE_TIME_REFRESH_MS,
    REMOVED_HOLD_MS,
    ROW_ANIMATION_MS,
    SORT_FREEZE_MS,
  } from '../live/timing';
  import type { DeviceItem } from './devices';
  import DeviceRow from './DeviceRow.svelte';

  let {
    net,
    items,
    label,
    selected = null,
    followFocus = false,
    stale,
    header = true,
    onOpen,
  }: {
    net: string;
    items: readonly DeviceItem[];
    label: string;
    selected?: string | null;
    followFocus?: boolean;
    stale?: boolean;
    header?: boolean;
    onOpen: (item: DeviceItem) => void;
  } = $props();

  let grid: HTMLElement | null = $state(null);
  let now = $state(live.now());
  let pointerInside = $state(false);
  let focusInside = $state(false);
  let frozen = $state(false);
  // Seeded from the items so the first frame shows every row (§8.1: no empty first paint).
  let order = $state<string[]>(untrack(() => items.map((i) => i.key)));
  const ghosts = new SvelteMap<string, DeviceItem>();
  const menus = new SvelteMap<string, boolean>();
  let activeKey = $state<string | null>(null);
  let typed = '';
  /** The last item seen per key, to show a removed device in place. Not reactive on purpose. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- read only inside an effect
  const lastItem = new Map<string, DeviceItem>();

  onMount(() => {
    const timer = setInterval(() => (now = live.now()), RELATIVE_TIME_REFRESH_MS);
    return () => clearInterval(timer);
  });

  // §8.7: hold the order while hovered or focused, and for SORT_FREEZE_MS after.
  $effect(() => {
    if (pointerInside || focusInside) {
      frozen = true;
      return;
    }
    const timer = setTimeout(() => (frozen = false), SORT_FREEZE_MS);
    return () => clearTimeout(timer);
  });

  // A device that left the network (not one a filter hid) stays as a `Removed` ghost.
  $effect(() => {
    const present = new Set(items.map((i) => i.key));
    const peers = live.peers[net] ?? {};
    for (const [key, item] of lastItem) {
      if (present.has(key)) continue;
      lastItem.delete(key);
      // Still in the network: a filter or the search hid it.
      if (item.self || item.peer.node in peers) continue;
      untrack(() => ghosts.set(key, item));
      setTimeout(() => ghosts.delete(key), REMOVED_HOLD_MS);
    }
    for (const item of items) lastItem.set(item.key, item);
  });

  $effect(() => {
    const keys = items.map((i) => i.key);
    const ghostKeys = [...ghosts.keys()];
    const hold = frozen;
    const previous = untrack(() => order);
    const known = new Set([...keys, ...ghostKeys]);
    if (hold) {
      const kept = previous.filter((k) => known.has(k));
      order = [...kept, ...keys.filter((k) => !kept.includes(k))];
      return;
    }
    const next = keys.slice();
    for (const key of ghostKeys) {
      if (next.includes(key)) continue;
      const at = previous.indexOf(key);
      next.splice(at < 0 ? next.length : Math.min(at, next.length), 0, key);
    }
    order = next;
  });

  const byKey = $derived(new Map(items.map((i) => [i.key, i])));
  const rows = $derived(
    order.flatMap((key) => {
      const item = byKey.get(key) ?? ghosts.get(key);
      return item ? [{ item, removed: !byKey.has(key) }] : [];
    }),
  );
  const tabStop = $derived.by(() => {
    const keys = rows.map((r) => r.item.key);
    if (activeKey && keys.includes(activeKey)) return activeKey;
    const sel = rows.find((r) => r.item.peer.node === selected);
    return sel?.item.key ?? keys[0] ?? null;
  });

  const reduced =
    typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  /** §3.5 row insert and remove: height plus fade, 160 ms. */
  function rowMotion(node: HTMLElement): TransitionConfig {
    const height = node.offsetHeight;
    return {
      duration: reduced ? 0 : ROW_ANIMATION_MS,
      css: (t) => `height: ${t * height}px; opacity: ${t}; overflow: hidden;`,
    };
  }

  function rowElement(key: string): HTMLElement | null {
    return grid?.querySelector<HTMLElement>(`[role='row'][data-key='${CSS.escape(key)}']`) ?? null;
  }

  function move(to: number): void {
    const target = rows[Math.max(0, Math.min(rows.length - 1, to))];
    if (!target || target.item.key === document.activeElement?.getAttribute('data-key')) return;
    activeKey = target.item.key;
    rowElement(target.item.key)?.focus();
    if (followFocus && !target.removed) onOpen(target.item);
  }

  function typeahead(char: string, from: number): void {
    const names = rows.map((r) => r.item.peer.display_name.toLowerCase());
    const next = typed + char;
    const startsWith = (prefix: string, i: number): boolean => names[i]?.startsWith(prefix) ?? false;
    // Extending the typed prefix keeps the current row if it still matches.
    if (typed !== '' && startsWith(next, from)) {
      typed = next;
      return;
    }
    for (const prefix of [next, char]) {
      for (let step = 1; step <= names.length; step += 1) {
        const i = (from + step) % names.length;
        if (startsWith(prefix, i)) {
          typed = prefix;
          move(i);
          return;
        }
      }
    }
    typed = '';
  }

  function keydown(event: KeyboardEvent): void {
    const target = event.target as HTMLElement | null;
    if (!target?.matches("[role='row']")) return;
    const index = rows.findIndex((r) => r.item.key === target.dataset['key']);
    if (index < 0) return;
    const plain = !event.metaKey && !event.ctrlKey && !event.altKey;
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        typed = '';
        move(index + 1);
        return;
      case 'ArrowUp':
        event.preventDefault();
        typed = '';
        move(index - 1);
        return;
      case 'Home':
        event.preventDefault();
        typed = '';
        move(0);
        return;
      case 'End':
        event.preventDefault();
        typed = '';
        move(rows.length - 1);
        return;
      default:
        if (plain && /^[\p{L}\p{N}-]$/u.test(event.key)) {
          event.preventDefault();
          typeahead(event.key.toLowerCase(), index);
        }
    }
  }

  function focusin(event: FocusEvent): void {
    focusInside = true;
    const row = (event.target as HTMLElement | null)?.closest<HTMLElement>("[role='row']");
    const key = row?.dataset['key'];
    if (key) activeKey = key;
  }

  function focusout(event: FocusEvent): void {
    const next = event.relatedTarget as Node | null;
    if (!next || !grid?.contains(next)) focusInside = false;
  }
</script>

<div class="devices">
  <div
    bind:this={grid}
    class="grid"
    role="grid"
    aria-label={label}
    aria-rowcount={rows.length + (header ? 1 : 0)}
    tabindex="-1"
    onkeydown={keydown}
    onfocusin={focusin}
    onfocusout={focusout}
    onpointerenter={() => (pointerInside = true)}
    onpointerleave={() => (pointerInside = false)}
  >
    {#if header}
      <div class="kith-device-cols head" role="row">
        <span class="c-dot" role="columnheader"><span class="sr-only">{copy.device.columns.status}</span></span>
        <span class="c-name" role="columnheader">{copy.device.columns.name}</span>
        <span class="c-path" role="columnheader">{copy.device.columns.path}</span>
        <span class="c-rtt" role="columnheader">{copy.device.columns.latency}</span>
        <span class="c-spark" role="columnheader">{copy.device.columns.traffic}</span>
        <span class="c-rate" role="columnheader">{copy.device.columns.rate}</span>
        <span class="c-seen" role="columnheader">{copy.device.columns.lastSeen}</span>
        <span class="c-more" role="columnheader"><span class="sr-only">{copy.device.columns.actions}</span></span>
      </div>
    {/if}
    {#each rows as row (row.item.key)}
      <div class="slot" role="none" in:rowMotion out:rowMotion>
        <DeviceRow
          {net}
          peer={row.item.peer}
          self={row.item.self}
          selected={row.item.peer.node === selected}
          active={row.item.key === tabStop}
          removed={row.removed}
          {stale}
          {now}
          onOpen={() => {
            if (!row.removed) onOpen(row.item);
          }}
          bind:menuOpen={
            () => menus.get(row.item.key) ?? false,
            (open) => menus.set(row.item.key, open)
          }
        />
      </div>
    {/each}
  </div>
</div>

<style>
  .devices {
    container: devices / inline-size;
  }
  .grid {
    display: grid;
    outline: none;
  }
  .slot {
    border-bottom: var(--hairline) solid var(--line);
  }
  .slot:last-child {
    border-bottom: 0;
  }
  .head {
    display: none;
    position: sticky;
    top: var(--sticky-top, 0);
    z-index: var(--z-header);
    min-height: var(--control);
    background: var(--bg);
    border-bottom: var(--hairline) solid var(--line);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    font-variant-caps: all-small-caps;
    color: var(--ink-3);
    cursor: default;
  }
  .head .c-rtt,
  .head .c-rate,
  .head .c-seen {
    text-align: right;
  }
  @media (min-width: 40rem) {
    @container devices (min-width: 528px) {
      .head {
        display: grid;
      }
    }
  }
</style>
