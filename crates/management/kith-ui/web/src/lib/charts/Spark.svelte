<!--
@component
Spark (spec §5): 120×24 rx sparkline drawn by the list's shared painter. Placeholder;
workstream F owns it.
Props:
  - peer: string, the `net/node` key into live.traffic.
-->
<script lang="ts">
  let { peer }: { peer: string } = $props();
</script>

<canvas class="spark" data-peer={peer} aria-hidden="true"></canvas>

<style>
  .spark {
    width: var(--spark-w);
    height: var(--spark-h);
  }
</style>
