<!--
@component
Headline (spec §6.2): one `--t-display` sentence that answers "is it working, does anything
need me?", with an optional inline action. The first matching condition wins: paused,
a network problem (in `--danger`, its fix as a link), exit offline, join requests (admin),
an expected device unreachable (`Why?`), only this device (`Add a device`), then
`Everything is working. 11 of 12 devices online.` with the sub-line
`9 direct · 2 via vps · ↓ 3.4 MB/s ↑ 220 kB/s`. Pause has no Resume button until the
endpoint exists (§2.17).
Props:
  - net: string, the network id.
-->
<script lang="ts">
  import { deviceName, exitName, isAdmin, networkName, peers, summary } from '../../frame/net';
  import { needs } from '../../frame/needs.svelte';
  import { go, href, openOverlay } from '../../router';
  import { copy, THIN } from '../copy';
  import type { Parts } from '../format';
  import CopyField from '../ui/CopyField.svelte';
  import { live } from '../live/store.svelte';
  import { TRAFFIC_WINDOW_MS } from '../live/timing';
  import type { PeerInfo } from '../types.gen';
  import { headline, type HeadlineAction } from './headline';

  let { net }: { net: string } = $props();

  /** Offline devices this device was using: its exit, or any traffic in the last hour. */
  function expected(list: readonly PeerInfo[], exit: string | null): PeerInfo[] {
    const now = live.now();
    void live.trafficVersion;
    return list
      .filter((p) => !p.online)
      .filter((p) => {
        if (p.name === exit) return true;
        const columns = live.traffic.get(`${net}/${p.node}`);
        if (!columns || columns.length < 2) return false;
        const last = columns.length - 1;
        const recent = now - (columns.at[last] ?? 0) < TRAFFIC_WINDOW_MS;
        return recent && (columns.rx[last] ?? 0) > (columns.rx[0] ?? 0);
      })
      .sort((a, b) => (a.last_seen_secs ?? Infinity) - (b.last_seen_secs ?? Infinity));
  }

  const view = $derived.by(() => {
    const list = peers(net);
    const exit = exitName();
    const relayOf: Record<string, string> = {};
    let down = 0;
    let up = 0;
    for (const p of list) {
      const numbers = live.numbers[`${net}/${p.node}`];
      if (!numbers) continue;
      down += numbers.rx;
      up += numbers.tx;
      if (numbers.relay) relayOf[p.node] = deviceName(net, numbers.relay);
    }
    const state = live.status?.exit_state ?? null;
    return headline({
      network: networkName(net),
      netId: net,
      admin: isAdmin(net),
      pausedUntilMs: live.status?.paused_until ?? null,
      problems: summary(net)?.problems ?? [],
      exit: exit && state ? { name: exit, state } : null,
      joins: needs.asking(net).map((n) => n.join.claimed.name),
      peers: list,
      expected: expected(list, exit),
      relayOf,
      down,
      up,
    });
  });

  function act(action: HeadlineAction): void {
    if (action.kind === 'add_device') openOverlay('add', 'device');
    else if (action.kind === 'why') go(href({ name: 'device', net, dev: action.node }));
    else if (action.target.kind === 'device') go(href({ name: 'device', net, dev: action.target.node }));
    else if (action.target.kind === 'this') go(href({ name: 'this' }));
  }
</script>

{#snippet amount(parts: Parts)}{parts.value}{#if parts.unit}{THIN}<span class="unit">{parts.unit}</span>{/if}{/snippet}

<div class="headline">
  <p class="sentence" class:danger={view.tone === 'danger'}>
    {view.text}
    {#if view.action && view.action.kind !== 'fix'}
      {@const action = view.action}
      <button type="button" class="inline" onclick={() => act(action)}>{action.label}</button>
    {/if}
  </p>
  {#if view.action?.kind === 'fix'}
    {@const action = view.action}
    {#if action.target.kind === 'command'}
      <p class="sub">{action.label}</p>
      <CopyField label={copy.gate.commandLabel} value={action.target.command} />
    {:else}
      <button type="button" class="fix" onclick={() => act(action)}>{action.label}</button>
    {/if}
  {/if}
  {#if view.subParts}
    {@const sub = view.subParts}
    <p class="sub">
      {sub.counts}{copy.overview.sep}↓ {@render amount(sub.down)} ↑ {@render amount(sub.up)}
    </p>
  {/if}
</div>

<style>
  .headline {
    display: grid;
    gap: var(--s-2);
    justify-items: start;
  }
  .sentence {
    font: var(--t-display);
    letter-spacing: var(--t-display-tracking);
    color: var(--ink-1);
    max-width: var(--measure);
    text-wrap: pretty;
  }
  .danger {
    color: var(--danger);
  }
  .inline,
  .fix {
    color: var(--ink-2);
    text-decoration: underline;
    text-decoration-thickness: var(--hairline);
    text-underline-offset: var(--s-1);
    cursor: pointer;
    border-radius: var(--r-1);
    transition: color var(--d-hover) var(--e-linear);
  }
  .inline {
    font: inherit;
    letter-spacing: inherit;
    font-weight: 400;
  }
  .inline:hover,
  .fix:hover {
    color: var(--ink-1);
  }
  .fix {
    font: var(--t-body);
    text-align: left;
  }
  .sub {
    color: var(--ink-2);
  }
  .unit {
    color: var(--ink-3);
  }
</style>
