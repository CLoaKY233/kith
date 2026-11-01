<!--
@component
Series (spec §5): the uPlot detail chart, mirrored rx/tx, event pins, relayed bands.
Placeholder; workstream F owns it.
Props:
  - peer: string, the `net/node` key into live.traffic.
  - range: Range ('1m' | '15m' | '1h').
  - events: readonly Entry[], this device's journal entries for pins.
-->
<script lang="ts">
  import { copy } from '../copy';
  import type { Range } from '../types';
  import type { Entry } from '../types.gen';

  let { peer, range, events }: { peer: string; range: Range; events: readonly Entry[] } =
    $props();
</script>

<figure class="series" role="img" aria-label={copy.chart.summary(peer, '', '')} data-range={range} data-events={events.length}></figure>

<style>
  .series {
    height: var(--series-h);
    background: var(--sunken);
    border-radius: var(--r-3);
  }
</style>
