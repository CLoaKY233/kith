<!--
@component
AddDevice (spec §7.3). Placeholder; workstream E owns it.
Props:
  - open: boolean, bindable. The screen maps it to the URL query (Back closes it).
  - net: string
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import Sheet from '../lib/ui/Sheet.svelte';

  let { open = $bindable(false), net }: { open?: boolean; net: string } = $props();
</script>

<Sheet bind:open title={copy.addDevice.title} closeLabel={copy.action.close}>
  <p hidden>{net}</p>
</Sheet>
