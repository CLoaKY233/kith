<!--
@component
Headline (spec §6.2). Placeholder; workstream D owns it.
Props:
  - net: string
-->
<script lang="ts">
  import { copy } from '../copy';

  let { net }: { net: string } = $props();
</script>

<p class="t-display" data-net={net}>{copy.overview.onlyThis}</p>
