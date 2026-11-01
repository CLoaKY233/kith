<!--
@component
ConnectionBanner (spec §5, §8.5). Placeholder; workstream D owns it.
Props:
  - state: ConnState
-->
<script lang="ts">
  import { copy } from '../copy';
  import type { ConnState } from '../types';

  let { state }: { state: ConnState } = $props();
</script>

{#if state === 'ui_gone'}<div class="banner" role="status">{copy.conn.uiGone}</div>{/if}
