<!--
@component
Rate (spec §5): a fixed 7ch slot, right aligned. Bytes SI, 3 significant figures; `idle`
below 1 kB/s in ink-3; `—` when stale. Members see a band for other devices' links.
Props:
  - value: number | null, bytes per second.
  - band?: RateBand | null, shown instead of a number when set.
  - stale?: boolean.
-->
<script lang="ts">
  import { copy } from '../copy';
  import { rate } from '../format';
  import { IDLE_BYTES_PER_SEC } from '../live/timing';
  import type { RateBand } from '../types.gen';

  let {
    value,
    band = null,
    stale = false,
  }: { value: number | null; band?: RateBand | null; stale?: boolean } = $props();

  const text = $derived(stale ? copy.units.none : band ? copy.rateBand[band] : rate(value));
  const quiet = $derived(stale || band === 'idle' || (value !== null && value < IDLE_BYTES_PER_SEC));
</script>

<span class="rate" class:quiet aria-live="off">{text}</span>

<style>
  .rate {
    display: inline-block;
    min-width: 7ch;
    text-align: right;
  }
  .quiet {
    color: var(--ink-3);
  }
</style>
