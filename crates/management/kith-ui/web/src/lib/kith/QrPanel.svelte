<!--
@component
QrPanel (spec §5 QrPanel). Placeholder; workstream E owns it.
Props:
  - svg: string | null
-->
<script lang="ts">
  import { copy } from '../copy';

  let { svg }: { svg: string | null } = $props();
</script>

<div class="qr" data-has={svg !== null}>{copy.qr.show}</div>
