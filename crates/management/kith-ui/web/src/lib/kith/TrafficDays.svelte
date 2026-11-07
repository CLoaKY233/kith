<!--
@component
TrafficDays: a device's traffic per day for the last 30 days, from what the Console kept
(`GET /api/history/traffic`), under the hour the chart shows. Read on request, since most
visits only want the chart.
Props:
  - network: string, the network id.
  - node: string, the device id.
-->
<script lang="ts">
  import { errorText } from '../api';
  import { copyConsole } from '../copy.console';
  import { bytes } from '../format';
  import { actions, live } from '../live/store.svelte';
  import Button from '../ui/Button.svelte';
  import { dayLabel } from './activity';
  import { dailyTotals, type DayTotal } from './trafficDays';

  let { network, node }: { network: string; node: string } = $props();

  /** As far back as the Console keeps traffic. */
  const KEPT_MS = 30 * 24 * 60 * 60 * 1000;

  let days = $state.raw<DayTotal[] | null>(null);
  let pending = $state(false);
  let error = $state<string | null>(null);

  async function show(): Promise<void> {
    if (pending) return;
    pending = true;
    error = null;
    const now = live.lastTickMs || live.now();
    const handle = await actions.trafficHistory(network, node, now - KEPT_MS).done;
    pending = false;
    if (handle.result) days = dailyTotals(handle.result, network, node);
    else if (handle.error) error = errorText(handle.error);
  }
</script>

<div class="days">
  {#if days === null}
    <div>
      <Button variant="ghost" {pending} pendingLabel={copyConsole.history.daysReading} onclick={show}>
        {copyConsole.history.daysShow}
      </Button>
    </div>
    {#if error}<p class="problem" role="alert">{error}</p>{/if}
  {:else if days.length === 0}
    <p class="meta">{copyConsole.history.daysNone}</p>
  {:else}
    <table>
      <caption class="sr-only">{copyConsole.history.daysCaption}</caption>
      <thead>
        <tr>
          <th scope="col">{copyConsole.history.daysDay}</th>
          <th scope="col">{copyConsole.history.daysReceived}</th>
          <th scope="col">{copyConsole.history.daysSent}</th>
        </tr>
      </thead>
      <tbody>
        {#each days as one (one.dayMs)}
          <tr>
            <th scope="row">{dayLabel(one.dayMs, live.lastTickMs || live.now())}</th>
            <td>{bytes(one.rx)}</td>
            <td>{bytes(one.tx)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .days {
    display: grid;
    gap: var(--s-2);
  }
  .meta {
    color: var(--ink-3);
  }
  .problem {
    color: var(--danger);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
  }
  th,
  td {
    padding: var(--s-1) var(--s-2);
    border-bottom: var(--hairline) solid var(--line);
    text-align: end;
  }
  th:first-child {
    text-align: start;
    font-weight: 400;
  }
  thead th {
    font: var(--t-small);
    color: var(--ink-3);
  }
</style>
