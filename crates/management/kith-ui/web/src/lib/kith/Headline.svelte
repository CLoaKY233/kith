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
  import { go, href, openOverlay } from '../../router';
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
      joins: live.pending.joins.filter((j) => j.network === net).map((j) => j.claimed.name),
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
    else openOverlay('need', action.need);
  }
</script>

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
    <button type="button" class="fix" onclick={() => act(action)}>{action.label}</button>
  {/if}
  {#if view.sub}<p class="sub">{view.sub}</p>{/if}
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
</style>
