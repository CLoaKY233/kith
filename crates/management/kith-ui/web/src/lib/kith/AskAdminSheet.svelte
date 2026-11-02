<!--
@component
AskAdminSheet (spec §7.10, §1 ruling 7): where a member would have had an admin control.
Names the network's admins with whether each is online, and gives the exact request to
send. There is no "sent" state: v1 has no member requests (§2.17).
Props:
  - open?: boolean, bindable. Ignored with `bare`.
  - net: string, the network's id or name.
  - request: string, what to send, from `copy.askAdmin.request.*`.
  - bare?: boolean. Render only the body, to sit inside another layer (the AllowBlocked
    popover) instead of opening a sheet of its own; no modal opens from another (§5).
  - onOpenChange?: (open: boolean) => void. Screens map it to the URL query.
-->
<script lang="ts">
  import { copy } from '../copy';
  import { live } from '../live/store.svelte';
  import CopyField from '../ui/CopyField.svelte';
  import Sheet from '../ui/Sheet.svelte';
  import { networkContext } from '../../sheets/act';

  let {
    open = $bindable(false),
    net,
    request,
    bare = false,
    onOpenChange,
  }: {
    open?: boolean;
    net: string;
    request: string;
    bare?: boolean;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const network = $derived(networkContext(live.status, net));
  const sendId = $props.id();

  /**
   * `desk (online), nas (offline)`, in the order the network lists its admins. This device
   * is not in its own device list, so it counts as online.
   */
  const admins = $derived.by(() => {
    if (!network) return '';
    const peers = Object.values(live.peers[network.id] ?? {});
    const self = network.membership?.name ?? null;
    return (network.membership?.admins ?? [])
      .map((name) =>
        copy.askAdmin.adminState(
          name,
          name === self || (peers.find((p) => p.name === name)?.online ?? false),
        ),
      )
      .join(', ');
  });
</script>

{#snippet body()}
  <div class="ask">
    {#if network}
      <p class="lede">{copy.askAdmin.onlyAdmins(network.name, admins)}</p>
    {/if}
    <div class="send">
      <p id={sendId} class="label">{copy.askAdmin.sendThis}</p>
      <CopyField value={request} labelledby={sendId} />
    </div>
  </div>
{/snippet}

{#if bare}
  {@render body()}
{:else}
  <Sheet bind:open title={copy.askAdmin.title} closeLabel={copy.action.close} {onOpenChange}>
    {@render body()}
  </Sheet>
{/if}

<style>
  .ask {
    display: grid;
    gap: var(--s-4);
    max-width: var(--measure);
  }
  .lede {
    color: var(--ink-2);
  }
  .send {
    display: grid;
    gap: var(--s-2);
  }
  .label {
    font-weight: 600;
  }
</style>
