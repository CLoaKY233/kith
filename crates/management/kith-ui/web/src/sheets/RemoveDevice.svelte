<!--
@component
RemoveDevice (spec §7.9, tier 2). Placeholder; workstream E owns it.
Props:
  - open: boolean, bindable.
  - net: string, network id. node: string, device id. name: string; network: string (its name).
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import AlertDialog from '../lib/ui/AlertDialog.svelte';

  let {
    open = $bindable(false),
    net,
    node,
    name,
    network,
  }: { open?: boolean; net: string; node: string; name: string; network: string } = $props();
</script>

<AlertDialog
  bind:open
  title={copy.remove.title(name, network)}
  description={copy.remove.loses(name)}
  cancelLabel={copy.action.cancel}
  confirmLabel={copy.remove.submit(name)}
  onConfirm={() => undefined}
>
  <p hidden>{net} {node}</p>
</AlertDialog>
