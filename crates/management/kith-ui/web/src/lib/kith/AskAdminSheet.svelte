<!--
@component
AskAdminSheet (spec §7.10). Placeholder; workstream E owns it.
Props:
  - net: string
  - request: string
-->
<script lang="ts">
  import { copy } from '../copy';

  let { net, request }: { net: string; request: string } = $props();
</script>

<div data-net={net}>{copy.askAdmin.sendThis} {request}</div>
