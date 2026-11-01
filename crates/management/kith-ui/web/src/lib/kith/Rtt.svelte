<!--
@component
Rtt (spec §5): a 6ch slot, right aligned: `12 ms`, `142 ms` or `—`. Only the text changes,
never with a counting animation. The one number that tints (§8.4): when the latency more
than doubles and stays there for 3 ticks, the slot gets the 4 s change tint once.
Props:
  - ms: number | null, the round-trip time.
  - stale?: boolean, shows `—` (§8.5).
-->
<script lang="ts">
  import { rtt } from '../format';
  import { live } from '../live/store.svelte';
  import { RTT_TINT_FACTOR, RTT_TINT_TICKS } from '../live/timing';

  let { ms, stale = false }: { ms: number | null; stale?: boolean } = $props();

  let baseline: number | null = null;
  let over = 0;
  let seenTick = 0;
  /** Bumped to restart the tint; 0 means never tinted. */
  let tints = $state(0);

  $effect(() => {
    const tick = live.lastTickMs;
    const value = ms;
    if (tick === seenTick) return;
    seenTick = tick;
    if (value === null || stale) {
      over = 0;
      return;
    }
    if (baseline === null) {
      baseline = value;
      return;
    }
    if (value > baseline * RTT_TINT_FACTOR) {
      over += 1;
      if (over >= RTT_TINT_TICKS) {
        tints += 1;
        baseline = value;
        over = 0;
      }
      return;
    }
    over = 0;
    baseline = value;
  });
</script>

{#key tints}
  <span class="rtt" class:tinted={tints > 0} aria-live="off">{rtt(stale ? null : ms)}</span>
{/key}

<style>
  .rtt {
    display: inline-block;
    min-width: 6ch;
    text-align: right;
    white-space: nowrap;
    border-radius: var(--r-1);
  }
  .tinted {
    animation: rtt-tint var(--d-tint) var(--e-linear);
  }
  @keyframes rtt-tint {
    from {
      background: var(--changed);
    }
  }
</style>
