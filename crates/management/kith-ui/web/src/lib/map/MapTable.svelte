<!--
@component
MapTable (spec §9.2): the map's visually hidden twin (from · to · path · latency); the
Grid view shows it. Placeholder; workstream F owns it.
Props:
  - net: string, network id.
  - visible?: boolean (default false: visually hidden).
-->
<script lang="ts">
  import { copy } from '../copy';

  let { net, visible = false }: { net: string; visible?: boolean } = $props();
</script>

<table class:sr-only={!visible} data-net={net}>
  <thead>
    <tr><th>{copy.map.from}</th><th>{copy.map.to}</th><th>{copy.map.path}</th><th>{copy.map.latency}</th></tr>
  </thead>
  <tbody></tbody>
</table>
