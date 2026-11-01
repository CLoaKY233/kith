<!--
@component
RoleChip (spec §5): 20 px, text only. Click opens a popover that explains the role.
Admin: ink-1 on sunken · Member: outline · View only: outline, ink-3 · Removed: danger on danger-wash.
The chip updates before the content when the network switches (it reads only its props).
Props:
  - role: Role ('admin' | 'member' | 'view_only' | 'removed')
  - network: string, the network's name.
  - keyFpr?: string, this device's key fingerprint (admin).
  - madeAdminBy?: string | null; madeAdminAtMs?: number | null.
  - admins?: readonly string[], the network's admins (member explanation).
-->
<script lang="ts">
  import { copy } from '../copy';
  import { shortDate } from '../format';
  import type { Role } from '../types.gen';
  import CopyField from './CopyField.svelte';
  import Popover from './Popover.svelte';

  let {
    role,
    network,
    keyFpr,
    madeAdminBy = null,
    madeAdminAtMs = null,
    admins = [],
  }: {
    role: Role;
    network: string;
    keyFpr?: string;
    madeAdminBy?: string | null;
    madeAdminAtMs?: number | null;
    admins?: readonly string[];
  } = $props();
</script>

<Popover>
  {#snippet trigger(props)}
    <button type="button" class="chip {role}" {...props}>{copy.role[role]}</button>
  {/snippet}
  <div class="explain">
    {#if role === 'admin'}
      <p>{copy.role.adminExplain(network)}</p>
      {#if keyFpr}<CopyField value={keyFpr} label={copy.role.key(keyFpr)} />{/if}
      {#if madeAdminBy && madeAdminAtMs !== null}
        <p class="meta">{copy.role.madeAdminBy(madeAdminBy, shortDate(madeAdminAtMs))}</p>
      {/if}
    {:else}
      <p>{copy.role.memberExplain(network, copy.role.orList(admins))}</p>
    {/if}
  </div>
</Popover>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    height: var(--chip);
    padding: 0 var(--s-2);
    border-radius: var(--r-1);
    font: var(--t-small);
    font-weight: 600;
    border: var(--hairline) solid var(--line-strong);
    color: var(--ink-1);
    cursor: pointer;
  }
  .admin {
    background: var(--sunken);
    border-color: transparent;
  }
  .view_only {
    color: var(--ink-3);
  }
  .removed {
    background: var(--danger-wash);
    border-color: transparent;
    color: var(--danger);
  }
  .explain {
    display: grid;
    gap: var(--s-2);
    max-width: var(--recent-column);
  }
  .meta {
    font: var(--t-small);
    color: var(--ink-3);
  }
</style>
