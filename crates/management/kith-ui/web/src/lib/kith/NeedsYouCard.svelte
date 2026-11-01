<!--
@component
NeedsYouCard (spec §5, §7.3, §7.5). Placeholder; workstream D owns it.
Props:
  - need: Need
-->
<script lang="ts">
  import { copy } from '../copy';
  import type { Need } from '../types';

  let { need }: { need: Need } = $props();
</script>

<article class="card" data-kind={need.kind}>{copy.overview.needsYou}</article>
