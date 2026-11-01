<!--
@component
ExitPicker (spec §6.7, §7.8). Placeholder; workstream E owns it.
Props:
  - net: string
-->
<script lang="ts">
  import { copy } from '../copy';

  let { net }: { net: string } = $props();
</script>

<div role="radiogroup" aria-label={copy.exits.heading} data-net={net}></div>
