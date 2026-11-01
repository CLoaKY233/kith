<!--
@component
EventRow (spec §5 EventRow, §6.5, §12.3): `56px time · 16px glyph · sentence · actor`.
The glyph shows only for blocked connections, warnings and signed changes. The actor is
`reported by laptop`, or `signed by laptop` followed by the seal. A row about a device
links to it with `?at=&range=` so the moment is in view (`range=1h` past the last hour).
Grouped rows (blocked connections, path flaps) and log lines expand on click. Every row
ends in a ⋯ menu (§11.4): `Open device`, `Copy line`, and `Allow this…` for a blocked
connection that is still pending (§7.5). Log lines are mono 12 px on one
line with an `INF` / `WRN` / `ERR` tag; `ERR` is in `--danger`.
Props:
  - entry: Entry. For a group, its newest entry.
  - group?: readonly Entry[] | null, the grouped entries, newest first.
  - expanded?: boolean, bindable. Whether a group or log line shows in full.
  - onToggle?: (expanded: boolean) => void, called when the person expands or collapses it.
  - link?: boolean (default true). False keeps the row from linking to the device.
  - fresh?: boolean. Arrived just now: grows in and carries the 4 s change tint (§8.4).
-->
<script lang="ts">
  import { go, href, openOverlay } from '../../router';
  import { copy, copyScreens } from '../copy';
  import { absolute, clock } from '../format';
  import Icon from '../icons/Icon.svelte';
  import { live } from '../live/store.svelte';
  import type { Entry } from '../types.gen';
  import DropdownMenu, { type MenuItem } from '../ui/DropdownMenu.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import { toast } from '../ui/Toast.svelte';
  import {
    describe,
    describeGroup,
    devicesOf,
    logLevel,
    logLine,
    rangeFor,
    type Glyph,
  } from './activity';
  import EventRow from './EventRow.svelte';
  import { liveContext } from './liveContext';

  let {
    entry,
    group = null,
    expanded = $bindable(false),
    onToggle,
    link = true,
    fresh = false,
  }: {
    entry: Entry;
    group?: readonly Entry[] | null;
    expanded?: boolean;
    onToggle?: (expanded: boolean) => void;
    link?: boolean;
    fresh?: boolean;
  } = $props();

  const ctx = liveContext;

  const grouped = $derived(group !== null && group.length > 1);
  const said = $derived(
    (grouped && group ? describeGroup(group, ctx) : null) ?? describe(entry, ctx),
  );
  const level = $derived(logLevel(entry));
  const isLog = $derived(entry.kind === 'log');

  /** The device this row opens, with the moment parked in view (§6.5 Linking). */
  const target = $derived.by(() => {
    if (!link || grouped || isLog) return null;
    const device = devicesOf(entry)[0];
    if (!device || entry.kind === 'device_removed' || entry.kind === 'join_requested') return null;
    const peer = live.peers[device.network]?.[device.node];
    const network = live.status?.networks.find((n) => n.id === device.network);
    if (!peer || !network) return null;
    const path = href({ name: 'device', net: network.name, dev: peer.name });
    const range = rangeFor(entry.at_ms, live.now()) ?? '1h';
    return `${path}?at=${entry.at_ms}&range=${range}`;
  });

  /** The device the ⋯ menu opens: the row's link, or for a group its newest entry's. */
  const deviceHref = $derived.by(() => {
    if (target) return target;
    if (isLog || entry.kind === 'device_removed' || entry.kind === 'join_requested') return null;
    const device = devicesOf(entry)[0];
    if (!device) return null;
    const peer = live.peers[device.network]?.[device.node];
    const network = live.status?.networks.find((n) => n.id === device.network);
    return peer && network ? href({ name: 'device', net: network.name, dev: peer.name }) : null;
  });

  /** §7.5: the pending blocked group this row is about, if it still needs someone. */
  const pendingBlocked = $derived.by(() => {
    if (entry.kind !== 'blocked') return null;
    const e = entry;
    return (
      live.pending.blocked.find(
        (g) =>
          g.network === e.network &&
          g.from === e.from &&
          g.to === e.to &&
          g.port === e.port &&
          g.protocol === e.protocol,
      ) ?? null
    );
  });

  function lineText(): string {
    const log = logLine(entry);
    if (log) return log;
    return [clock(entry.at_ms), said.sentence, said.actor].filter(Boolean).join(' ');
  }

  async function copyLine(): Promise<void> {
    try {
      await navigator.clipboard.writeText(lineText());
      toast.done(copyScreens.activity.lineCopied);
    } catch {
      toast.failed(copyScreens.activity.copyFailed);
    }
  }

  const menu: MenuItem[] = $derived([
    ...(deviceHref
      ? [{ label: copyScreens.activity.openDevice, onSelect: () => go(deviceHref) }]
      : []),
    { label: copyScreens.activity.copyLine, onSelect: () => void copyLine() },
    ...(pendingBlocked
      ? [
          {
            label: copy.blocked.allowThis,
            onSelect: () => openOverlay('need', pendingBlocked.id),
          },
        ]
      : []),
  ]);

  const glyphLabel: Record<Glyph, string> = {
    ban: copyScreens.activity.glyphs.blocked,
    'triangle-alert': copyScreens.activity.glyphs.warning,
    seal: copyScreens.activity.glyphs.signed,
  };

  const iso = (ms: number): string => new Date(ms).toISOString();

  function toggle(): void {
    expanded = !expanded;
    onToggle?.(expanded);
  }

  function open(event: MouseEvent): void {
    if (!target || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
      return;
    }
    event.preventDefault();
    go(target);
  }
</script>

{#snippet time(ms: number)}
  <time class="time" datetime={iso(ms)} title={absolute(ms)}>{clock(ms)}</time>
{/snippet}

{#snippet body()}
  {@render time(entry.at_ms)}
  <span class="glyph">
    {#if said.glyph && !said.sealed}<Icon name={said.glyph} label={glyphLabel[said.glyph]} />{/if}
  </span>
  <span class="sentence">
    {said.sentence}
    {#if grouped}
      <span class="chevron"><Icon name={expanded ? 'chevron-down' : 'chevron-right'} /></span>
    {/if}
  </span>
  {#if said.actor}
    <span class="actor">
      {said.actor}
      {#if said.sealed}<Icon name="seal" label={copyScreens.activity.glyphs.signed} />{/if}
    </span>
  {/if}
{/snippet}

{#snippet more()}
  <span class="more">
    <DropdownMenu items={menu} label={copyScreens.activity.rowMore}>
      {#snippet trigger(props)}
        <IconButton icon="ellipsis" label={copyScreens.activity.rowMore} triggerProps={props} />
      {/snippet}
    </DropdownMenu>
  </span>
{/snippet}

<div class="event" class:fresh class:log={isLog}>
  <div class="inner">
    {#if isLog && level}
      <div class="row">
        <button type="button" class="line log-line" aria-expanded={expanded} onclick={toggle}>
          {@render time(entry.at_ms)}
          <span class="tag" class:err={level === 'error'}>{copy.activity.levels[level]}</span>
          <span class="message" class:open={expanded}>
            {said.sentence}
            {#if expanded && entry.kind === 'log'}<span class="source">{entry.target}</span>{/if}
          </span>
        </button>
        {@render more()}
      </div>
    {:else if grouped && group}
      <div class="row">
        <button type="button" class="line" aria-expanded={expanded} onclick={toggle}>
          {@render body()}
        </button>
        {@render more()}
      </div>
      {#if expanded}
        <ol class="children">
          {#each group as child (child.seq)}
            <li><EventRow entry={child} {link} /></li>
          {/each}
        </ol>
      {/if}
    {:else if target}
      <div class="row">
        <a class="line" href={target} onclick={open}>{@render body()}</a>
        {@render more()}
      </div>
    {:else}
      <div class="row">
        <div class="line">{@render body()}</div>
        {@render more()}
      </div>
    {/if}
  </div>
</div>

<style>
  /* §3.5: a new row grows in (height plus fade) and carries the change tint, which fades
     by opacity on a layer of its own so reduced motion keeps it. */
  .event {
    position: relative;
    display: grid;
    grid-template-rows: 1fr;
    border-radius: var(--r-2);
  }
  .inner {
    min-height: 0;
  }
  .event.fresh {
    animation: event-grow var(--d-row) var(--e-linear);
  }
  .event.fresh > .inner {
    overflow: hidden;
  }
  .event.fresh::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: var(--changed);
    opacity: 0;
    pointer-events: none;
    animation: event-tint var(--d-tint) var(--e-linear);
  }
  @keyframes event-grow {
    from {
      grid-template-rows: 0fr;
      opacity: 0;
    }
  }
  @keyframes event-tint {
    from {
      opacity: 1;
    }
  }

  .row {
    display: flex;
    align-items: flex-start;
    gap: var(--s-1);
  }
  .row > .line {
    flex: 1;
    min-width: 0;
  }
  .more {
    display: inline-flex;
    flex: none;
    padding-top: calc((var(--row) - var(--control)) / 2);
  }

  .line {
    display: grid;
    grid-template-columns: calc(var(--s-7) + var(--s-2)) var(--icon) minmax(0, 1fr) auto;
    grid-template-areas: 'time glyph sentence actor';
    align-items: baseline;
    column-gap: var(--s-3);
    width: 100%;
    min-height: var(--row);
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-2);
    color: var(--ink-1);
    text-align: left;
    text-decoration: none;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  a.line,
  button.line {
    cursor: pointer;
  }
  a.line:hover,
  button.line:hover {
    background: var(--sunken);
  }
  .line:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: calc(-1 * var(--focus-ring));
  }

  .time {
    grid-area: time;
    color: var(--ink-3);
    font: var(--t-small);
  }
  .glyph {
    grid-area: glyph;
    display: inline-flex;
    align-self: start;
    padding-top: calc((1lh - var(--icon)) / 2);
    color: var(--ink-2);
  }
  .sentence {
    grid-area: sentence;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .chevron {
    display: inline-flex;
    vertical-align: middle;
    color: var(--ink-3);
  }
  .actor {
    grid-area: actor;
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--ink-3);
    font: var(--t-small);
    white-space: nowrap;
  }

  .log-line {
    grid-template-columns: calc(var(--s-7) + var(--s-2)) 3ch minmax(0, 1fr);
    grid-template-areas: 'time tag message';
    align-items: baseline;
  }
  .tag {
    grid-area: tag;
    color: var(--ink-3);
    font: var(--t-log);
  }
  .tag.err {
    color: var(--danger);
  }
  .message {
    grid-area: message;
    min-width: 0;
    overflow: hidden;
    font: var(--t-log);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .message.open {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .source {
    display: block;
    color: var(--ink-3);
  }

  .children {
    margin-left: calc(var(--s-7) + var(--s-2) + var(--s-3));
    padding-left: var(--s-3);
    border-left: var(--hairline) solid var(--line);
  }

  @media (max-width: 40rem) {
    .line:not(.log-line) {
      grid-template-columns: calc(var(--s-7) + var(--s-2)) var(--icon) minmax(0, 1fr);
      grid-template-areas:
        'time glyph sentence'
        '. . actor';
      row-gap: var(--s-1);
    }
    .actor {
      white-space: normal;
    }
    .children {
      margin-left: var(--s-3);
    }
  }
</style>
