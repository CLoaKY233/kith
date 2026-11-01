<!--
@component
RangeControl (spec §5 Series): `1m · 15m · 1h` as a Segmented. The choice lives in the URL
(`?range=`) and replaces the history entry, so every chart on the screen shares it and
Back leaves the screen instead of stepping through ranges.
Props:
  - range: Range, the current value (read it with rangeFrom(query)).
-->
<script lang="ts">
  import { setQuery } from '../../router';
  import { copy } from '../copy';
  import type { Range } from '../types';
  import Segmented from '../ui/Segmented.svelte';
  import { RANGES } from './range';

  let { range }: { range: Range } = $props();

  const options = RANGES.map((r) => ({ value: r, label: copy.chart.ranges[r] }));
</script>

<Segmented
  value={range}
  {options}
  label={copy.chart.rangeLabel}
  onValueChange={(next) => setQuery({ range: next }, { replace: true })}
/>
