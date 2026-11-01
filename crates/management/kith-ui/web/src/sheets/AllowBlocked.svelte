<!--
@component
AllowBlocked (spec §7.5). Placeholder; workstream E owns it.
Props:
  - open: boolean, bindable. The screen maps it to the URL query (Back closes it).
  - id: string
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import Sheet from '../lib/ui/Sheet.svelte';

  let { open = $bindable(false), id }: { open?: boolean; id: string } = $props();
</script>

<Sheet bind:open title={copy.blocked.allowThis} closeLabel={copy.action.close}>
  <p hidden>{id}</p>
</Sheet>
