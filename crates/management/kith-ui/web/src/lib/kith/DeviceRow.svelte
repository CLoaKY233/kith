<!--
@component
DeviceRow (spec §5): the core row, 44 px, one per device. Render it inside DeviceList (or
any `role="grid"` that is a `devices` size container); the container's width picks the
columns, so the same row works in the full list, beside the split detail and on a phone:
  full    dot · name + mono address · path · latency · spark · rate · last seen · ⋯
  tablet  drops the address and last seen
  narrow  also drops the spark (the list beside the detail panel)
  phone   two lines, 64 px: dot, name, path on the right; then `100.64.0.7 · 34 ms · ↓ 1.2 MB/s`
States: hover (sunken) · focus (inset ring) · selected (sunken plus a 2 px ink-1 bar) ·
offline (ink-3, hollow dot, `seen 3 h ago`) · never seen (`not connected yet`, `added 2 min
ago`) · waiting phone (dashed dot, `waiting for first connection`) · changed (4 s tint) ·
needs you (2 px attn bar on attn-wash) · in progress (a flow's text in place, such as
`Removing…`) · removed (`Removed`, then the list collapses it) · stale (`—`, last-known dot).
The accessible name is computed on focus ("nas, online, direct, 3 milliseconds, receiving
1.20 megabytes per second"); number cells are aria-live=off.
Props:
  - net: string, network id.
  - peer: PeerInfo.
  - self?: boolean, this device: tagged `you`, pinned first by the list.
  - selected?: boolean, its detail is open.
  - active?: boolean, the grid's roving tab stop (tabindex 0).
  - removed?: boolean, the device was just removed: the row reads `Removed`.
  - stale?: boolean, values are last known (§8.5); defaults to the store's connection.
  - now?: number, server ms for relative times; the list passes a 1 s clock.
  - onOpen?: () => void, opens the detail (click, Enter).
  - menuOpen?: boolean, bindable: the ⋯ menu (`.` opens it).
-->
<script lang="ts">
  import Spark from '../charts/Spark.svelte';
  import { copy } from '../copy';
  import { rate as formatRate, rtt as formatRtt } from '../format';
  import { live } from '../live/store.svelte';
  import { PHONE_ABANDONED_MS } from '../live/timing';
  import type { DotState } from '../types';
  import type { PeerInfo } from '../types.gen';
  import Badge from '../ui/Badge.svelte';
  import DropdownMenu, { type MenuItem } from '../ui/DropdownMenu.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import StatusDot from '../ui/StatusDot.svelte';
  import { toast } from '../ui/Toast.svelte';
  import { setQuery } from '../../router';
  import { renameRequest, rowProgress, snapshotSeenAt } from './deviceState.svelte';
  import {
    agoMs,
    blockedInvolving,
    facts,
    fqdn,
    isFlapping,
    joinedAt,
    lastSeenAt,
    pathText,
    presenceOf,
    relayName,
    spokenRate,
  } from './devices';
  import PathBadge from './PathBadge.svelte';
  import Rate from './Rate.svelte';
  import Rtt from './Rtt.svelte';

  let {
    net,
    peer,
    self = false,
    selected = false,
    active = false,
    removed = false,
    stale,
    now,
    onOpen,
    menuOpen = $bindable(false),
  }: {
    net: string;
    peer: PeerInfo;
    self?: boolean;
    selected?: boolean;
    active?: boolean;
    removed?: boolean;
    stale?: boolean;
    now?: number;
    onOpen?: () => void;
    menuOpen?: boolean;
  } = $props();

  const key = $derived(`${net}/${peer.node}`);
  const clock = $derived(now ?? live.now());
  const isStale = $derived(stale ?? (live.conn === 'stale' || live.conn === 'daemon_down'));
  const numbers = $derived(live.numbers[key] ?? null);
  const network = $derived(live.status?.networks.find((n) => n.id === net) ?? null);
  const admin = $derived(network?.admin ?? false);
  const relay = $derived(relayName(live.peers[net], numbers?.relay ?? null));
  const seenAt = $derived(
    lastSeenAt(live.journal, net, peer, snapshotSeenAt(key, peer.last_seen_secs, clock)),
  );
  const addedAt = $derived(joinedAt(live.journal, net, peer.node));
  const presence = $derived(self ? ({ kind: 'online' } as const) : presenceOf(peer, seenAt, addedAt));
  const progress = $derived(rowProgress.get(key) ?? null);
  const flapping = $derived(peer.online && isFlapping(live.journal, net, peer.node, clock));
  const changed = $derived(live.changed.has(key));
  const needsYou = $derived(
    admin &&
      !self &&
      !removed &&
      (blockedInvolving(live.pending.blocked, net, peer.node) !== null ||
        (presence.kind === 'waiting' &&
          addedAt !== null &&
          clock - addedAt > PHONE_ABANDONED_MS)),
  );

  const dot: DotState = $derived.by(() => {
    if (removed) return 'offline';
    if (presence.kind === 'waiting') return 'waiting';
    if (presence.kind !== 'online') return 'offline';
    return isStale ? 'stale' : 'online';
  });
  const statusWord = $derived(
    presence.kind === 'online' ? (isStale ? copy.status.stale : copy.status.online) : copy.status.offline,
  );

  /** The words in the path slot when the device has no path to show. */
  const note: string | null = $derived.by(() => {
    if (removed) return copy.device.removed;
    if (progress) return progress;
    switch (presence.kind) {
      case 'online':
        return null;
      case 'offline':
        return copy.device.seenAgo(agoMs(clock - presence.seenAt));
      case 'never':
        return copy.device.notConnectedYet;
      case 'waiting':
        return copy.device.waitingFirst;
    }
  });
  /**
   * The last-seen column. Online means something authenticated arrived moments ago, and
   * the store holds no fresher time than that, so an online device reads `now`.
   */
  const seenText = $derived.by(() => {
    if (removed) return '';
    switch (presence.kind) {
      case 'online':
        return copy.time.now;
      case 'offline':
        return copy.device.seenAgo(agoMs(clock - presence.seenAt));
      case 'never':
      case 'waiting':
        return presence.addedAt === null
          ? ''
          : copy.deviceView.addedAgo(agoMs(clock - presence.addedAt));
    }
  });

  const rx = $derived(isStale || presence.kind !== 'online' ? null : (numbers?.rx ?? null));
  const rttMs = $derived(presence.kind === 'online' ? (numbers?.rttMs ?? null) : null);
  /** Line 2 of the phone layout. */
  const line2 = $derived(
    presence.kind === 'online' && !removed && !progress
      ? facts([
          peer.address,
          self || rttMs === null ? null : formatRtt(isStale ? null : rttMs),
          self ? null : `${copy.units.down} ${formatRate(rx)}`,
        ])
      : facts([peer.address, presence.kind === 'online' ? null : seenText]),
  );

  let label = $state('');
  function describe(): void {
    const parts: string[] = [peer.display_name];
    if (self) parts.push(copy.device.you);
    if (removed) parts.push(copy.device.removed);
    else if (progress) parts.push(progress);
    else parts.push(statusWord);
    if (presence.kind === 'online' && !self && !removed) {
      const path = pathText(peer.path, relay);
      if (path) parts.push(path);
      if (rttMs !== null && !isStale) parts.push(copy.spoken.milliseconds(Math.round(rttMs)));
      if (rx !== null) parts.push(copy.spoken.receiving(spokenRate(rx)));
    } else if (note && note !== progress) {
      parts.push(note);
    }
    label = copy.device.rowLabel(parts);
  }

  async function write(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      toast.done(copy.copyField.copied);
    } catch {
      // Clipboard refused (no permission); nothing was copied, so nothing to say.
    }
  }

  const menu: MenuItem[] = $derived.by(() => {
    const name = network?.name ?? '';
    const items: MenuItem[] = [
      { label: copy.deviceView.copyAddress, onSelect: () => void write(peer.address) },
      { label: copy.deviceView.copyName, onSelect: () => void write(fqdn(peer.name, name)) },
    ];
    // Nothing is queued while stale (§8.5), so the menu offers only what needs no daemon.
    if (removed || isStale) return items;
    if (admin) {
      items.push({ separator: true });
      items.push({
        label: copy.detail.rename,
        onSelect: () => {
          renameRequest.key = key;
          onOpen?.();
        },
      });
      if (!self) {
        items.push({
          label: copy.detail.remove(name),
          tone: 'danger',
          onSelect: () => setQuery({ remove: peer.name }),
        });
      }
    } else if (!self) {
      items.push({ separator: true });
      items.push({ label: copy.detail.askAdmin, onSelect: () => onOpen?.() });
    }
    return items;
  });

  function keydown(event: KeyboardEvent): void {
    if (event.target !== event.currentTarget) return;
    if (event.key === 'Enter') {
      event.preventDefault();
      onOpen?.();
    } else if (event.key === '.') {
      event.preventDefault();
      menuOpen = true;
    }
  }
</script>

<div
  class="kith-device-cols row"
  class:offline={presence.kind !== 'online' || removed}
  class:selected
  class:needs={needsYou}
  class:changed
  role="row"
  aria-keyshortcuts="."
  tabindex={active ? 0 : -1}
  aria-selected={selected}
  aria-label={label || peer.display_name}
  data-key={key}
  data-name={peer.name}
  onfocus={describe}
  onclick={() => onOpen?.()}
  onkeydown={keydown}
>
  <span class="c-dot" role="gridcell"><StatusDot state={dot} /></span>
  <span class="c-name" role="gridcell">
    <span class="name">{peer.display_name}</span>
    {#if self}<Badge>{copy.device.you}</Badge>{/if}
    <span class="addr">{peer.address}</span>
  </span>
  <span class="c-path" role="gridcell">
    {#if note !== null}
      <span
        class="note"
        class:offline-note={presence.kind === 'offline' && !removed && !progress}
        title={note}>{note}</span
      >
    {:else if !self}
      <PathBadge
        path={peer.path}
        {relay}
        {flapping}
        endpoint={peer.endpoint}
        mtu={peer.path_mtu}
        reason={peer.path_reason}
        focusable={false}
      />
    {/if}
  </span>
  <span class="c-rtt" role="gridcell">
    {#if !self && presence.kind === 'online' && !removed}<Rtt ms={rttMs} stale={isStale} />{/if}
  </span>
  <span class="c-spark" role="gridcell">
    {#if !self && (presence.kind === 'online' || presence.kind === 'offline') && !removed}
      <Spark peer={key} />
    {/if}
  </span>
  <span class="c-rate" role="gridcell">
    {#if !self && presence.kind === 'online' && !removed}
      <Rate value={rx} stale={isStale} />
    {/if}
  </span>
  <span class="c-seen" role="gridcell" title={seenText}>{seenText}</span>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <span class="c-more" role="gridcell" tabindex="-1" onclick={(event) => event.stopPropagation()}>
    <DropdownMenu items={menu} bind:open={menuOpen}>
      {#snippet trigger(props)}
        <IconButton
          icon="ellipsis"
          label={copy.deviceView.rowMenu(peer.display_name)}
          tabindex={-1}
          triggerProps={props}
        />
      {/snippet}
    </DropdownMenu>
  </span>
  <span class="c-facts" aria-hidden="true">{line2}</span>
</div>

<style>
  /*
   * Columns are picked by the width of the nearest `devices` container (DeviceList), so
   * the list's column header shares them: it uses the same global class. Container
   * widths are in px because `rem` there follows the root's 14 px body font: 528 px fits
   * the narrow columns, 660 px adds the spark, 984 px (the content width at 1280 px) is full.
   */
  :global(.kith-device-cols) {
    display: grid;
    align-items: center;
    column-gap: var(--s-3);
    padding-inline: var(--s-3);
    /* §5: 20px dot … 32px ⋯; the ⋯ column grows with the 44 px touch target. */
    --more: var(--control);
    grid-template-columns: 20px auto minmax(0, 1fr) var(--more);
    grid-template-areas:
      'dot name path more'
      '. facts spark more';
    min-height: var(--row-mobile);
  }
  @media (pointer: coarse) {
    :global(.kith-device-cols) {
      --more: var(--target-touch);
    }
  }
  :global(.kith-device-cols > .c-dot) {
    grid-area: dot;
  }
  :global(.kith-device-cols > .c-name) {
    grid-area: name;
  }
  :global(.kith-device-cols > .c-path) {
    grid-area: path;
    justify-self: end;
  }
  :global(.kith-device-cols > .c-spark) {
    grid-area: spark;
  }
  :global(.kith-device-cols > .c-more) {
    grid-area: more;
  }
  :global(.kith-device-cols > .c-facts) {
    grid-area: facts;
  }
  :global(.kith-device-cols > .c-rtt),
  :global(.kith-device-cols > .c-rate),
  :global(.kith-device-cols > .c-seen) {
    display: none;
  }
  :global(.kith-device-cols .addr) {
    display: none;
  }

  @media (min-width: 40rem) {
    /* narrow: beside the split detail. */
    @container devices (min-width: 528px) {
      :global(.kith-device-cols) {
        min-height: var(--row);
        grid-template-columns: 20px minmax(0, 1fr) 150px 64px 84px var(--more);
        grid-template-areas: 'dot name path rtt rate more';
      }
      :global(.kith-device-cols > .c-path) {
        justify-self: start;
      }
      :global(.kith-device-cols > .c-rtt) {
        display: block;
        grid-area: rtt;
        text-align: right;
      }
      :global(.kith-device-cols > .c-rate) {
        display: block;
        grid-area: rate;
        text-align: right;
      }
      :global(.kith-device-cols > .c-facts),
      :global(.kith-device-cols > .c-spark) {
        display: none;
      }
    }
    /* tablet: drops the address and last seen. */
    @container devices (min-width: 660px) {
      :global(.kith-device-cols) {
        grid-template-columns: 20px minmax(0, 1fr) 150px 64px var(--spark-w) 84px var(--more);
        grid-template-areas: 'dot name path rtt spark rate more';
      }
      :global(.kith-device-cols > .c-spark) {
        display: block;
      }
    }
    /* full: every column. */
    @container devices (min-width: 984px) {
      :global(.kith-device-cols) {
        grid-template-columns: 20px minmax(0, 1fr) 150px 64px var(--spark-w) 84px 96px var(--more);
        grid-template-areas: 'dot name path rtt spark rate seen more';
      }
      :global(.kith-device-cols > .c-seen) {
        display: block;
        grid-area: seen;
      }
      :global(.kith-device-cols .addr) {
        display: inline;
      }
      /* `seen 3 h ago` moves from the path slot to its own column. */
      :global(.kith-device-cols .offline-note) {
        display: none;
      }
    }
  }

  .row {
    position: relative;
    cursor: pointer;
    outline: none;
    color: var(--ink-1);
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .row:hover {
    background: var(--sunken);
  }
  .row:focus-visible {
    box-shadow: inset 0 0 0 var(--focus-ring) var(--accent);
  }
  .row.selected {
    background: var(--sunken);
  }
  .row.selected::before,
  .row.needs::before {
    content: '';
    position: absolute;
    inset-block: 0;
    left: 0;
    width: var(--lead-bar);
    background: var(--ink-1);
  }
  .row.needs {
    background: var(--attn-wash);
  }
  .row.needs::before {
    background: var(--attn);
  }
  .row.changed::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: var(--changed);
    opacity: 0;
    animation: row-tint var(--d-tint) var(--e-linear);
  }
  @keyframes row-tint {
    from {
      opacity: 1;
    }
  }
  .offline {
    color: var(--ink-3);
  }

  .c-dot {
    display: flex;
    align-items: center;
  }
  .c-name {
    display: flex;
    align-items: baseline;
    gap: var(--s-2);
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .addr {
    font: var(--t-small);
    font-family: var(--ff-mono);
    color: var(--ink-3);
    white-space: nowrap;
  }
  .c-path {
    display: flex;
    min-width: 0;
  }
  .note {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--ink-3);
  }
  .c-seen {
    font: var(--t-small);
    color: var(--ink-3);
    text-align: right;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .c-facts {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--t-small);
    color: var(--ink-2);
  }
  .offline .c-facts {
    color: var(--ink-3);
  }
  .c-spark {
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .c-more {
    display: flex;
    justify-content: center;
  }
</style>
