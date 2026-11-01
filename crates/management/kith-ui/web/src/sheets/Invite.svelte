<!--
@component
Invite (spec §7.4): choose how long the link lasts and how many devices may use it, then
`Create invite` (tier 1: no extra confirmation). The result shows the link, its QR behind a
click, the command to run and whether it has been used; `Revoke invite` is instant.
Props:
  - open: boolean, bindable. The screen maps it to the URL query (Back closes it).
  - net: string, the network as the route names it.
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import { live } from '../lib/live/store.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import { findNetwork, isStale } from './flow';
  import { InviteFlow } from './inviteFlow.svelte';
  import InviteBody from './InviteBody.svelte';

  let { open = $bindable(false), net }: { open?: boolean; net: string } = $props();

  const network = $derived(findNetwork(net));
  const flow = new InviteFlow(() => network?.id ?? null, () => live.now());
  const stale = $derived(isStale());

  // Each time the sheet opens it offers a fresh invite.
  $effect(() => {
    if (open) flow.reset();
  });
</script>

<Sheet bind:open title={copy.invite.title} closeLabel={copy.action.close}>
  {#if network}
    <InviteBody {flow} networkId={network.id} />
  {/if}

  {#snippet footer()}
    {#if flow.invite === null}
      {#if stale}<p class="reason">{copy.conn.staleActions}</p>{/if}
      <Button
        variant="primary"
        pending={flow.pending}
        disabled={stale}
        onclick={() => flow.create()}
      >
        {copy.invite.create}
      </Button>
    {:else}
      <Button variant="secondary" onclick={() => (open = false)}>{copy.action.close}</Button>
    {/if}
  {/snippet}
</Sheet>

<style>
  .reason {
    flex: 1;
    align-self: center;
    font: var(--t-small);
    color: var(--ink-3);
  }
</style>
