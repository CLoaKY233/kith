<!--
@component
Device (spec §6.4): `/n/:net/devices/:dev`. The detail always opens beside, over or instead
of the Devices list (§4.3, §4.4), so this route is the Devices screen with a device open.
Props:
  - net: string, the network as the URL names it.
  - dev: string, the device as the URL names it.
  - query: URLSearchParams (`?range=1m|15m|1h &at=<ms>` plus the list's own).
-->
<script lang="ts">
  import Devices from './Devices.svelte';

  let { net, dev, query }: { net: string; dev: string; query: URLSearchParams } = $props();
</script>

<Devices {net} {query} {dev} />
