<!--
@component
Rtt (spec §5): a 6ch slot, `12 ms` or `—`.
Props:
  - ms: number | null.
  - stale?: boolean.
-->
<script lang="ts">
  import { rtt } from '../format';

  let { ms, stale = false }: { ms: number | null; stale?: boolean } = $props();
</script>

<span class="rtt" aria-live="off">{rtt(stale ? null : ms)}</span>

<style>
  .rtt {
    display: inline-block;
    min-width: 6ch;
    text-align: right;
  }
</style>
