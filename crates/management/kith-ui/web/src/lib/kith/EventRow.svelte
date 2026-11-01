<!--
@component
EventRow (spec §5 EventRow, §12.3). Placeholder; workstream D owns it.
Props:
  - entry: Entry
-->
<script lang="ts">
  import type { Entry } from '../types.gen';

  let { entry }: { entry: Entry } = $props();
</script>

<div class="event" data-seq={entry.seq}>{entry.kind}</div>
