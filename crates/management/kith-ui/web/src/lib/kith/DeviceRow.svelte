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
  import { connection } from '../../frame/conn.svelte';
  import { needs } from '../../frame/needs.svelte';
  import { removalLine, removalOf } from '../../sheets/removals.svelte';
  import { announce } from '../announce';
  import Spark from '../charts/Spark.svelte';
  import { copy, DOT } from '../copy';
  import { rate as formatRate, rtt as formatRtt } from '../format';
  import { live } from '../live/store.svelte';
  import { IDLE_BYTES_PER_SEC, PHONE_ABANDONED_MS } from '../live/timing';
  import type { DotState } from '../types';
  import type { PeerInfo } from '../types.gen';
  import Badge from '../ui/Badge.svelte';
  import DropdownMenu, { type MenuItem } from '../ui/RowMenu.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import StatusDot from '../ui/StatusDot.svelte';
  import { toast } from '../ui/Toast.svelte';
  import { setQuery } from '../../router';
  import { askRequest, renameRequest, rowProgress, snapshotSeenAt } from './deviceState.svelte';
  import {
    agoMs,
    facts,
    fqdn,
    isFlapping,
    joinedAt,
    lastSeenAt,
    pathText,
    presenceOf,
    isPhone,
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
  const isStale = $derived(stale ?? connection.stale);
  /** Nothing can change (§8.5): admin items stay, aria-disabled, with the reason. */
  const frozen = $derived(isStale || connection.frozen);
  const numbers = $derived(live.numbers[key] ?? null);
  const network = $derived(live.status?.networks.find((n) => n.id === net) ?? null);
  const admin = $derived(network?.admin ?? false);
  const selfName = $derived(
    live.status?.memberships.find((m) => m.network === net)?.name ?? null,
  );
  /** A phone hangs off its gateway: the path reads `via desk` (`via this device`). */
  const gateway = $derived(
    peer.gateway === null
      ? null
      : peer.gateway === selfName
        ? copy.device.thisDevice
        : peer.gateway,
  );
  const relay = $derived(gateway ?? relayName(live.peers[net], numbers?.relay ?? null));
  const path = $derived(gateway !== null ? ('relay' as const) : peer.path);
  const seenAt = $derived(
    lastSeenAt(live.journal, net, peer, snapshotSeenAt(key, peer.last_seen_secs, clock)),
  );
  const addedAt = $derived(joinedAt(live.journal, net, peer.node));
  const presence = $derived(self ? ({ kind: 'online' } as const) : presenceOf(peer, seenAt, addedAt));
  const removal = $derived(removalOf(net, peer.node));
  const progress = $derived(removal ? removalLine(removal) : (rowProgress.get(key) ?? null));
  // §11.1: the person hears where their removal ended up.
  $effect(() => {
    const seen = removal?.handle.seen;
    if (removal && seen && seen.by >= seen.of) announce(removalLine(removal));
  });
  const flapping = $derived(peer.online && isFlapping(live.journal, net, peer.node, clock));
  /** The tint's stamp: a second change inside 4 s restarts the fade (§8.4). */
  const stamp = $derived(live.changed.get(key) ?? null);
  // The shared needs list, so an ignored blocked card stops marking the row.
  const blockedNeed = $derived(
    needs.live.some(
      (n) =>
        n.kind === 'blocked' &&
        n.network === net &&
        (n.blocked.from === peer.node || n.blocked.to === peer.node),
    ),
  );
  const waitingTooLong = $derived(
    presence.kind === 'waiting' && addedAt !== null && clock - addedAt > PHONE_ABANDONED_MS,
  );
  const needsYou = $derived(admin && !self && !removed && (blockedNeed || waitingTooLong));
  /** §11.5: the needs-you state has a word, not only a colour. */
  const needsWord = $derived(
    needsYou ? (blockedNeed ? copy.device.needsBlocked : copy.device.needsWaiting) : null,
  );
  /** Members see `idle / light / busy` for other devices' links (§5 Rate). */
  const band = $derived(admin || self ? null : (numbers?.band ?? null));

  const dot: DotState = $derived.by(() => {
    if (removed) return 'offline';
    if (presence.kind === 'waiting') return 'waiting';
    if (presence.kind !== 'online') return 'offline';
    return isStale ? 'stale' : 'online';
  });
  const statusWord = $derived(
    presence.kind === 'online' ? (isStale ? copy.status.stale : copy.status.online) : copy.status.offline,
  );

  /** `just now` under a second, so an offline device never reads `seen now`. */
  function since(ms: number): string {
    return ms < 1000 ? copy.join.justNow : agoMs(ms);
  }

  /** The words in the path slot when the device has no path to show. */
  const note: string | null = $derived.by(() => {
    if (removed) return copy.device.removed;
    if (progress) return progress;
    switch (presence.kind) {
      case 'online':
        return null;
      case 'offline':
        return copy.device.seenAgo(since(clock - presence.seenAt));
      case 'never':
        return copy.device.notConnectedYet;
      case 'waiting':
        return copy.device.waitingFirst;
    }
  });
  /**
   * The last-seen column. Online means something authenticated arrived moments ago, so it
   * stays empty and the offline `seen 3 h ago` stands out.
   */
  const seenText = $derived.by(() => {
    if (removed) return '';
    switch (presence.kind) {
      case 'online':
        return '';
      case 'offline':
        return copy.device.seenAgo(since(clock - presence.seenAt));
      case 'never':
      case 'waiting':
        return presence.addedAt === null
          ? ''
          : copy.deviceView.addedAgo(agoMs(clock - presence.addedAt));
    }
  });

  const rx = $derived(isStale || presence.kind !== 'online' ? null : (numbers?.rx ?? null));
  const rttMs = $derived(presence.kind === 'online' ? (numbers?.rttMs ?? null) : null);
  /** Below 1 kB/s the rate reads `idle`, with no arrow (§5 Rate). */
  const rateFact = $derived.by(() => {
    if (band) return copy.rateBand[band];
    if (rx !== null && rx < IDLE_BYTES_PER_SEC) return copy.units.idle;
    return `${copy.units.down} ${formatRate(rx)}`;
  });
  /** Line 2 of the phone layout, after the mono address. */
  const line2 = $derived(
    presence.kind === 'online' && !removed && !progress
      ? facts([
          self || rttMs === null ? null : formatRtt(isStale ? null : rttMs),
          self ? null : rateFact,
        ])
      : // An offline row already says `seen 3 h ago` in its path slot.
        facts([presence.kind === 'online' || presence.kind === 'offline' ? null : seenText]),
  );

  let label = $state('');
  function describe(): void {
    const parts: string[] = [peer.display_name];
    if (self) parts.push(copy.device.you);
    if (removed) parts.push(copy.device.removed);
    else if (progress) parts.push(progress);
    else parts.push(statusWord);
    if (needsWord) parts.push(needsWord);
    if (presence.kind === 'online' && !self && !removed) {
      const text = pathText(path, relay);
      if (text) parts.push(text);
      if (rttMs !== null && !isStale) parts.push(copy.spoken.milliseconds(Math.round(rttMs)));
      if (band) parts.push(copy.rateBand[band]);
      else if (rx !== null) parts.push(copy.spoken.receiving(spokenRate(rx)));
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

  /** A member's ask: the detail opens with the Ask-an-admin sheet for it (§7.10). */
  function ask(request: string): void {
    askRequest.key = key;
    askRequest.request = request;
    onOpen?.();
  }

  const menu: MenuItem[] = $derived.by(() => {
    const name = network?.name ?? '';
    const items: MenuItem[] = [
      { label: copy.deviceView.copyAddress, onSelect: () => void write(peer.address) },
      { label: copy.deviceView.copyName, onSelect: () => void write(fqdn(peer.name, name)) },
    ];
    if (removed) return items;
    // §8.5: while frozen the changes stay, aria-disabled, with the reason as the last line.
    const disabled = frozen;
    if (admin) {
      items.push({ separator: true });
      items.push({
        label: copy.detail.rename,
        disabled,
        onSelect: () => {
          renameRequest.key = key;
          onOpen?.();
        },
      });
      if (!self) {
        items.push({
          label: copy.detail.remove(name),
          tone: 'danger',
          disabled,
          onSelect: () => setQuery({ remove: peer.name }),
        });
      }
    } else if (!self) {
      items.push({ separator: true });
      items.push({
        label: copy.deviceView.askRemove(peer.name, name),
        disabled,
        onSelect: () => ask(copy.askAdmin.request.remove(name, peer.name)),
      });
      if (!isPhone(peer)) {
        items.push({
          label: copy.deviceView.askAllowExit(peer.name),
          disabled,
          onSelect: () => ask(copy.askAdmin.request.exitAllow(name, peer.name)),
        });
      }
    }
    if (disabled && items.length > 2) {
      items.push({ label: copy.conn.staleActions, disabled: true, onSelect: () => {} });
    }
    return items;
  });

  let rowEl: HTMLElement | null = $state(null);
  let moreEl: HTMLButtonElement | null = $state(null);

  function keydown(event: KeyboardEvent): void {
    if (event.target !== event.currentTarget) return;
    if (event.key === 'Enter') {
      event.preventDefault();
      onOpen?.();
    } else if (
      event.key === '.' ||
      event.key === 'ContextMenu' ||
      (event.key === 'F10' && event.shiftKey)
    ) {
      event.preventDefault();
      menuOpen = true;
    } else if (event.key === 'ArrowRight') {
      // The ⋯ button is out of the tab order (one stop per grid); → reaches it.
      event.preventDefault();
      moreEl?.focus();
    }
  }

  function keydownMore(event: KeyboardEvent): void {
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      rowEl?.focus();
    }
  }
</script>

<div
  bind:this={rowEl}
  class="kith-device-cols row"
  class:offline={presence.kind !== 'online' || removed}
  class:selected
  class:needs={needsYou}
  role="row"
  tabindex={active ? 0 : -1}
  aria-selected={selected}
  aria-label={label || peer.display_name}
  aria-keyshortcuts="."
  data-key={key}
  data-name={peer.name}
  onfocus={describe}
  onclick={() => onOpen?.()}
  onkeydown={keydown}
>
  {#key stamp}
    {#if stamp !== null}<span class="tint" aria-hidden="true"></span>{/if}
  {/key}
  <span class="c-dot" role="gridcell">
    <StatusDot state={dot} />
    <span class="sr-only">{statusWord}</span>
  </span>
  <span class="c-name" role="gridcell">
    <span class="name">{peer.display_name}</span>
    {#if self}<Badge>{copy.device.you}</Badge>{/if}
    {#if needsWord}<span class="needs-word">{needsWord}</span>{/if}
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
        {path}
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
      <Rate value={rx} {band} stale={isStale} />
    {/if}
  </span>
  <span class="c-seen" role="gridcell" title={seenText}>{seenText}</span>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <span class="c-more" role="gridcell" tabindex="-1" onclick={(event) => event.stopPropagation()}>
    <DropdownMenu
      items={menu}
      bind:open={menuOpen}
      onCloseAutoFocus={(event) => {
        // Back to the row, so arrow keys keep moving through the grid (§5, §10).
        event.preventDefault();
        rowEl?.focus();
      }}
    >
      {#snippet trigger(props)}
        <IconButton
          bind:ref={moreEl}
          icon="ellipsis"
          label={copy.deviceView.rowMenu(peer.display_name)}
          onkeydown={keydownMore}
          triggerProps={{ ...props, tabindex: -1 }}
        />
      {/snippet}
    </DropdownMenu>
  </span>
  <span class="c-facts" aria-hidden="true"
    ><span class="facts-addr">{peer.address}</span>{#if line2}{DOT}{line2}{/if}</span
  >
</div>

<style>
  /*
   * Columns are picked by the width of the nearest `devices` container (DeviceList), so
   * the list's column header shares them: it uses the same global class. Container
   * widths are in px because `rem` there follows the root's 14 px body font: 528 px fits
   * the narrow columns, 600 px adds the spark (the tablet list), 984 px (the content width
   * at 1280 px) is full. Every cell's `display` lives in these global rules, never in a
   * scoped one: a scoped rule has the same specificity and comes later, so it would undo
   * a tier's `display: none`.
   */
  :global(.kith-device-cols) {
    display: grid;
    align-items: center;
    column-gap: var(--s-3);
    padding-inline: var(--s-3);
    /* §5: 20px dot … 32px ⋯; the ⋯ column grows with the 44 px touch target. */
    --more: var(--control);
    /* Line 2's facts never size a column, so every row's spark has one width and edge. */
    grid-template-columns: 20px minmax(0, 1fr) auto var(--more);
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
    display: flex;
    align-items: center;
    justify-self: end;
    min-width: 0;
  }
  :global(.kith-device-cols > .c-more) {
    grid-area: more;
    align-self: start;
  }
  :global(.kith-device-cols > .c-facts) {
    grid-area: facts;
    display: block;
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
      :global(.kith-device-cols > .c-more) {
        align-self: center;
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
    @container devices (min-width: 600px) {
      :global(.kith-device-cols) {
        column-gap: var(--s-2);
        grid-template-columns: 20px minmax(0, 1fr) 150px 64px var(--spark-w) 84px var(--more);
        grid-template-areas: 'dot name path rtt spark rate more';
      }
      :global(.kith-device-cols > .c-spark) {
        display: flex;
        justify-self: start;
        /* Room between the right-aligned latency and the spark, on header and rows. */
        margin-inline-start: var(--s-2);
      }
    }
    /* full: every column. */
    @container devices (min-width: 984px) {
      :global(.kith-device-cols) {
        column-gap: var(--s-3);
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
  /* Selection and hover stay visible on a needs-you row (§5 DeviceRow states). */
  .row.needs.selected::before {
    background: var(--ink-1);
  }
  .row.needs.selected,
  .row.needs:hover {
    background: color-mix(in srgb, var(--attn-wash), var(--ink-1) 4%);
  }
  .tint {
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
    /* Tabular figures would give the hyphen in `dev-box` a figure's width. */
    font-variant-numeric: slashed-zero;
  }
  .needs-word {
    flex: none;
    font: var(--t-small);
    color: var(--attn-text);
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
  /* ink-3 is under 4.5:1 on sunken and attn-wash; these rows use ink-2 (§11.3). */
  .row:hover .addr,
  .row:hover .c-seen,
  .row:hover .note,
  .row.selected .addr,
  .row.selected .c-seen,
  .row.selected .note,
  .row.needs .addr,
  .row.needs .c-seen,
  .row.needs .note {
    color: var(--ink-2);
  }
  .c-facts {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--t-small);
    color: var(--ink-2);
  }
  .facts-addr {
    font-family: var(--ff-mono);
  }
  .offline .c-facts {
    color: var(--ink-3);
  }
  .c-more {
    display: flex;
    justify-content: center;
  }
</style>
