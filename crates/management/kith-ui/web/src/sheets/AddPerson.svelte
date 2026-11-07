<!--
@component
AddPerson (docs/plans/console.md, flow 3): a name and a role, then a one-time sign-up link
to copy or scan. The owner may add admins; adding one is an authority change, so the
owner's passkey confirms it before the link shows. Admins add members only.
Props:
  - open: boolean, bindable.
  - onAdded?: () => void, after a person is added (the list reloads).
-->
<script lang="ts">
  import { me } from '../lib/account.svelte';
  import { AuthorityPrompt } from '../lib/authority.svelte';
  import { errorText } from '../lib/api';
  import { copy } from '../lib/copy';
  import { copyConsole } from '../lib/copy.console';
  import QrPanel from '../lib/kith/QrPanel.svelte';
  import { actions } from '../lib/live/store.svelte';
  import { addableRoles } from '../lib/people';
  import type { PersonAdded, PersonRole } from '../lib/types';
  import Button from '../lib/ui/Button.svelte';
  import CopyField from '../lib/ui/CopyField.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';

  let { open = $bindable(false), onAdded }: { open?: boolean; onAdded?: () => void } = $props();

  let phase = $state<'form' | 'confirm' | 'link'>('form');
  let name = $state('');
  let role = $state<PersonRole>('member');
  let nameError = $state<string | null>(null);
  let error = $state<string | null>(null);
  let pending = $state(false);
  let added = $state<PersonAdded | null>(null);
  const authority = new AuthorityPrompt();

  const roles = $derived(addableRoles(me()));
  const roleOptions = $derived(roles.map((r) => ({ value: r, label: copy.account.role[r] })));

  $effect(() => {
    if (!open) return;
    phase = 'form';
    name = '';
    role = 'member';
    nameError = error = null;
    pending = false;
    added = null;
  });

  function checkName(): boolean {
    nameError = name.trim() === '' ? copyConsole.people.nameEmpty : null;
    return nameError === null;
  }

  async function create(event?: SubmitEvent): Promise<void> {
    event?.preventDefault();
    if (pending || !checkName()) return;
    pending = true;
    error = null;
    const handle = await actions.addPerson(name.trim(), role).done;
    pending = false;
    if (handle.state === 'failed' && handle.error) {
      error = errorText(handle.error);
      return;
    }
    if (!handle.result) return;
    added = handle.result;
    onAdded?.();
    if (role === 'admin') {
      authority.prepare({ kind: 'add_admin', person: handle.result.id });
      phase = 'confirm';
    } else {
      phase = 'link';
    }
  }

  async function confirmAdmin(): Promise<void> {
    if (authority.state === 'signing') return;
    if (await authority.confirm()) {
      onAdded?.();
      phase = 'link';
    }
  }

  const shownName = $derived(name.trim());
</script>

<Sheet bind:open title={copyConsole.people.add} closeLabel={copy.action.close} footer={footer}>
  {#if phase === 'form'}
    <form id="add-person" class="body" onsubmit={create} novalidate>
      <Field label={copyConsole.people.nameLabel} error={nameError}>
        {#snippet children(control)}
          <Input
            {...control}
            bind:value={name}
            autocomplete="off"
            onblur={checkName}
            oninput={() => nameError !== null && checkName()}
          />
        {/snippet}
      </Field>
      {#if roles.length > 1}
        <div class="role">
          <Segmented bind:value={() => role, (v) => (role = v as PersonRole)} options={roleOptions} label={copyConsole.people.roleLabel} />
          <p class="helper">{copyConsole.people.roleHelper[role]}</p>
        </div>
      {:else}
        <p class="helper">{copyConsole.people.joinsAsMember}</p>
      {/if}
      {#if error}<p class="problem" role="alert">{error}</p>{/if}
    </form>
  {:else if phase === 'confirm'}
    <div class="body">
      <p>{copyConsole.people.confirmAdminLede(shownName)}</p>
      {#if authority.error}<p class="problem" role="alert">{authority.error}</p>{/if}
    </div>
  {:else if added}
    <div class="body">
      <h3 class="heading">{copyConsole.people.linkFor(shownName)}</h3>
      <p class="helper">{copyConsole.people.linkLede}</p>
      <CopyField value={added.link} label={copyConsole.people.linkLabel} />
      {#if added.qr_svg}<QrPanel svg={added.qr_svg} />{/if}
    </div>
  {/if}
</Sheet>

{#snippet footer()}
  {#if phase === 'form'}
    <Button variant="primary" type="submit" form="add-person" {pending} pendingLabel={copyConsole.people.creating}>
      {copyConsole.people.createLink}
    </Button>
  {:else if phase === 'confirm'}
    <Button variant="primary" pending={authority.state === 'signing'} onclick={confirmAdmin}>
      {copyConsole.people.confirmAdmin}
    </Button>
  {:else}
    <Button variant="primary" onclick={() => (open = false)}>{copyConsole.people.done}</Button>
  {/if}
{/snippet}

<style>
  .body,
  .role {
    display: grid;
    gap: var(--s-3);
  }
  .body {
    gap: var(--s-4);
  }
  .heading {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
  }
  .helper {
    color: var(--ink-3);
  }
  .problem {
    color: var(--danger);
  }
</style>
