<!--
@component
RoleChip (spec §5): 20 px, text only.
  Admin: `--ink-1` on `--sunken` · Member: outline · View only: outline, `--ink-3` ·
  Removed: `--danger` text on `--danger-wash`.
Admin and Member are buttons that open a popover explaining the role:
  Admin: "This device signs changes to home with its own key. Every other device checks
  the signature." · `Key k7f3·9a21·… Copy` · `Made admin by desk · 12 Oct`.
  Member: "This device can see home. Changes need an admin: desk or nas."
View only and Removed have nothing to explain and are plain text.
The chip reads only its props, so it updates before the content when the network switches.
Props:
  - role: Role ('admin' | 'member' | 'view_only' | 'removed').
  - network: string, the network's name.
  - keyFpr?: string, this device's key fingerprint (admin), shown shortened and copied whole.
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

  /** The fingerprint's first two groups, as `k7f3·9a21·`; the copy adds the ellipsis. */
  const SHOWN_GROUPS = 2;
  const shortFpr = $derived(
    keyFpr === undefined
      ? ''
      : keyFpr
          .split('·')
          .slice(0, SHOWN_GROUPS)
          .map((group) => `${group}·`)
          .join(''),
  );
  const explains = $derived(role === 'admin' || role === 'member');
</script>

{#if explains}
  <Popover label={copy.role[role]}>
    {#snippet trigger(props)}
      <button type="button" class="chip {role}" {...props}>{copy.role[role]}</button>
    {/snippet}
    <div class="explain">
      {#if role === 'admin'}
        <p>{copy.role.adminExplain(network)}</p>
        {#if keyFpr}<CopyField value={keyFpr} display={copy.role.key(shortFpr)} />{/if}
        {#if madeAdminBy && madeAdminAtMs !== null}
          <p class="meta">{copy.role.madeAdminBy(madeAdminBy, shortDate(madeAdminAtMs))}</p>
        {/if}
      {:else}
        <p>{copy.role.memberExplain(network, copy.role.orList(admins))}</p>
      {/if}
    </div>
  </Popover>
{:else}
  <span class="chip {role}">{copy.role[role]}</span>
{/if}

<style>
  .chip {
    display: inline-flex;
    flex: none;
    align-items: center;
    height: var(--chip);
    padding: 0 var(--s-2);
    border: var(--hairline) solid var(--line-strong);
    border-radius: var(--r-1);
    background: transparent;
    color: var(--ink-1);
    font: var(--t-small);
    font-weight: 600;
    white-space: nowrap;
    -webkit-tap-highlight-color: transparent;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  button.chip {
    cursor: pointer;
  }
  button.chip:hover,
  button.chip[aria-expanded='true'] {
    background: var(--sunken);
  }
  button.chip:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  /* Transparent rather than none, so forced colours still draw the chip. */
  .admin {
    background: var(--sunken);
    border-color: transparent;
  }
  button.admin:hover,
  button.admin[aria-expanded='true'] {
    background: var(--line);
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
    gap: var(--s-3);
    max-width: var(--recent-column);
  }
  .meta {
    font: var(--t-small);
    color: var(--ink-3);
  }
</style>
