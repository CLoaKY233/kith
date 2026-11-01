<!--
@component
IdentityBlock (spec §4.2). Placeholder; workstream D owns it.
Props:
  - net: string | null
-->
<script lang="ts">
  import { copy } from '../copy';

  let { net }: { net: string | null } = $props();
</script>

<div class="identity" data-net={net}>{copy.nav.thisDevice}</div>
