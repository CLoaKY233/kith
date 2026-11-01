<!--
@component
DeviceDetail (spec §6.4). Placeholder; workstream D owns it.
Props:
  - net: string
  - node: string
-->
<script lang="ts">
  import { copy } from '../copy';

  let { net, node }: { net: string; node: string } = $props();
</script>

<section data-net={net} data-node={node}>{copy.detail.path}</section>
