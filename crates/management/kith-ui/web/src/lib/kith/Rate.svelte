<!--
@component
Rate (spec §5): a fixed 9ch slot (`1.24 MB/s` fits), right aligned. Bytes SI, 3 significant figures; `idle`
below 1 kB/s in ink-3; `—` when stale. Members see a band for other devices' links.
Props:
  - value: number | null, bytes per second.
  - band?: RateBand | null, shown instead of a number when set.
  - stale?: boolean.
Decision (§5): rates use bytes so every number on the page shares one unit family.
-->
<script lang="ts">
  import { copy } from '../copy';
  import { rateParts } from '../format';
  import { IDLE_BYTES_PER_SEC } from '../live/timing';
  import type { RateBand } from '../types.gen';

  let {
    value,
    band = null,
    stale = false,
  }: { value: number | null; band?: RateBand | null; stale?: boolean } = $props();

  const parts = $derived(
    stale
      ? { value: copy.units.none, unit: '' }
      : band
        ? { value: copy.rateBand[band], unit: '' }
        : rateParts(value),
  );
  // §3.3: the unit follows a thin space in ink-3, so the number reads first.
  const quiet = $derived(stale || band === 'idle' || (value !== null && value < IDLE_BYTES_PER_SEC));
</script>

<!-- Each value renders as new text: right-aligned digits of another width would otherwise
     count as moved text (§11.9 CLS). -->
<span class="rate" class:quiet aria-live="off"
  >{#key `${parts.value} ${parts.unit}`}{parts.value}{#if parts.unit}&thinsp;<span class="unit"
        >{parts.unit}</span
      >{/if}{/key}</span
>

<style>
  .rate {
    display: inline-block;
    width: 9ch;
    font-variant-numeric: tabular-nums;
    text-align: right;
    white-space: nowrap;
  }
  .unit {
    color: var(--ink-3);
  }
  .quiet {
    color: var(--ink-3);
  }
</style>
