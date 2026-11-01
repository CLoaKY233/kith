<!--
@component
JoinNetwork (spec §7.2). Placeholder; workstream E owns it.
Props:
  - open: boolean, bindable. The screen maps it to the URL query (Back closes it).

-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import Sheet from '../lib/ui/Sheet.svelte';

  let { open = $bindable(false) }: { open?: boolean } = $props();
</script>

<Sheet bind:open title={copy.joinNetwork.title} closeLabel={copy.action.close}>
  
</Sheet>
