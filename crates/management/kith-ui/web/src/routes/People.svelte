<!--
@component
People (docs/plans/console.md): everyone in the network with their role and devices, and
the people invited. Owners and admins add people (a one-time link). Someone who made
their passkey waits at the top with one `Approve…` button: approving signs their
`AddAccount` with the approver's passkey, which an invited admin needs on top of the
confirmation made with their link. Roles change and people go through an AlertDialog that
says what changes, then the person's passkey confirms it. An invite nobody approved is
taken back at once: nothing was signed. Members see the list read-only, with who to ask.
Props:
  - net: string, the network id.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { me } from '../lib/account.svelte';
  import { AuthorityPrompt } from '../lib/authority.svelte';
  import { errorText } from '../lib/api';
  import { copy } from '../lib/copy';
  import { copyConsole } from '../lib/copy.console';
  import { actions, live, onEntry } from '../lib/live/store.svelte';
  import { addableRoles, canApprove, canRemove, roleChoices, runsNetwork, sortPeople } from '../lib/people';
  import type { AuthorityChangeIn, PersonView } from '../lib/types.gen';
  import AlertDialog from '../lib/ui/AlertDialog.svelte';
  import Button from '../lib/ui/Button.svelte';
  import DropdownMenu, { type MenuItem } from '../lib/ui/DropdownMenu.svelte';
  import IconButton from '../lib/ui/IconButton.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import { membership, networkName } from '../frame/net';
  import AddPerson from '../sheets/AddPerson.svelte';

  let { net }: { net: string } = $props();

  let people = $state.raw<PersonView[] | null>(null);
  let loadError = $state<string | null>(null);
  let adding = $state(false);

  type Change = Extract<AuthorityChangeIn, { kind: 'add_admin' | 'set_role' | 'remove_person' }>;
  type Pending = { readonly person: PersonView; readonly change: Change };
  let changing = $state.raw<Pending | null>(null);
  let dialogOpen = $state(false);
  const authority = new AuthorityPrompt();

  const viewer = $derived(me());
  const sorted = $derived(people ? sortPeople(people) : []);
  const admins = $derived(sorted.filter((p) => runsNetwork(p.role) && p.state === 'active').map((p) => p.name));
  const network = $derived(networkName(net));

  async function load(): Promise<void> {
    const handle = await actions.loadPeople().done;
    if (handle.state === 'idle') return;
    if (handle.result) {
      people = handle.result;
      loadError = null;
    } else if (handle.error) {
      loadError = errorText(handle.error);
    }
  }

  /** How often the list is read again while someone may be making their passkey. */
  const INVITED_POLL_MS = 10_000;

  onMount(() => {
    void load();
    // Making a passkey writes no journal entry, so an open invite is checked on a timer.
    const timer = setInterval(() => {
      if (document.visibilityState === 'visible' && people?.some((p) => p.state === 'invited')) void load();
    }, INVITED_POLL_MS);
    // A device joining or leaving changes someone's list.
    const stop = onEntry((entry) => {
      if (entry.kind === 'device_joined' || entry.kind === 'device_removed') void load();
    });
    return () => {
      clearInterval(timer);
      stop();
    };
  });

  /** A device by id, named and with its state from the live store. */
  function device(node: string): { name: string; online: boolean } {
    if (node === live.status?.node) return { name: membership(net)?.name ?? node.slice(0, 8), online: true };
    const peer = live.peers[net]?.[node];
    return { name: peer?.name ?? node.slice(0, 8), online: peer?.online ?? false };
  }

  function ask(person: PersonView, change: Change): void {
    changing = { person, change };
    authority.prepare(change);
    dialogOpen = true;
  }

  async function takeBack(person: PersonView): Promise<void> {
    const handle = await actions.cancelInvite(person.id).done;
    if (handle.state === 'failed' && handle.error) {
      toast.failed(errorText(handle.error));
      return;
    }
    toast.done(copyConsole.people.inviteCancelled(person.name));
    void load();
  }

  function menu(person: PersonView): MenuItem[] {
    const items: MenuItem[] = roleChoices(viewer, person).map((role) => ({
      label: role === 'admin' ? copyConsole.people.makeAdmin : copyConsole.people.makeMember,
      onSelect: () => ask(person, { kind: 'set_role', person: person.id, role }),
    }));
    if (canRemove(viewer, person)) {
      if (items.length > 0) items.push({ separator: true });
      items.push(
        person.state === 'active'
          ? {
              label: copyConsole.people.remove,
              tone: 'danger',
              onSelect: () => ask(person, { kind: 'remove_person', person: person.id }),
            }
          : { label: copyConsole.people.cancelInvite, tone: 'danger', onSelect: () => void takeBack(person) },
      );
    }
    return items;
  }

  const dialog = $derived.by(() => {
    if (!changing) return null;
    const { person, change } = changing;
    if (change.kind === 'add_admin') {
      return {
        title: copyConsole.people.approveTitle(person.name),
        body: copyConsole.people.approveBody(person.name, person.role, network),
        confirm: copyConsole.people.approveLabel(person.name),
        variant: 'primary' as const,
      };
    }
    if (change.kind === 'remove_person') {
      return {
        title: copyConsole.people.removeTitle(person.name),
        body: copyConsole.people.removeBody(person.name, person.devices.length, network),
        confirm: copyConsole.people.removeConfirm(person.name),
        variant: 'danger' as const,
      };
    }
    const admin = change.kind === 'set_role' && change.role === 'admin';
    return {
      title: admin ? copyConsole.people.makeAdminTitle(person.name) : copyConsole.people.makeMemberTitle(person.name),
      body: admin ? copyConsole.people.makeAdminBody(person.name, network) : copyConsole.people.makeMemberBody(person.name, network),
      confirm: admin ? copyConsole.people.makeAdmin : copyConsole.people.makeMember,
      variant: 'primary' as const,
    };
  });

  async function confirm(): Promise<void> {
    const current = changing;
    if (!current || authority.state === 'signing') return;
    if (!(await authority.confirm())) return;
    dialogOpen = false;
    const { person, change } = current;
    toast.done(
      change.kind === 'set_role'
        ? copyConsole.people.roleChanged(person.name, change.role)
        : change.kind === 'add_admin'
          ? copyConsole.people.approved(person.name)
          : copyConsole.people.removed(person.name),
    );
    void load();
  }
</script>

<section class="screen">
  {#if addableRoles(viewer).length > 0}
    <div class="toolbar">
      <Button variant="primary" onclick={() => (adding = true)}>{copyConsole.people.add}</Button>
    </div>
  {:else if viewer}
    <p class="ask">{copyConsole.people.askAdmin(admins)}</p>
  {/if}

  {#if loadError}
    <div class="failed">
      <p class="problem" role="alert">{copyConsole.people.loadFailed}</p>
      <Button onclick={load}>{copyConsole.people.retry}</Button>
    </div>
  {:else if people}
    <ul class="list" aria-label={copyConsole.people.title}>
      {#each sorted as person (person.id)}
        {@const items = menu(person)}
        <li class="person" class:waiting={person.state === 'registered'} data-state={person.state}>
          <div class="who">
            <span class="name">{person.name}</span>
            {#if person.me}<span class="you">{copyConsole.people.you}</span>{/if}
            <span class="role {person.role}">{copy.account.role[person.role]}</span>
          </div>
          <div class="devices">
            {#if person.state === 'registered'}
              <span class="note">
                <StatusDot state="needs_you" />
                {copyConsole.people.waiting(person.role)}
              </span>
              {#if canApprove(viewer, person)}
                <Button variant="primary" onclick={() => ask(person, { kind: 'add_admin', person: person.id })}>
                  {copyConsole.people.approve}
                </Button>
              {/if}
            {:else if person.state === 'invited'}
              <span class="quiet">{copyConsole.people.pending}</span>
            {:else if person.devices.length === 0}
              <span class="quiet">{copyConsole.people.noDevices}</span>
            {:else}
              {#each person.devices as node (node)}
                {@const shown = device(node)}
                <span class="device">
                  <StatusDot state={shown.online ? 'online' : 'offline'} />
                  {copyConsole.people.device(shown.name, shown.online)}
                </span>
              {/each}
            {/if}
          </div>
          <span class="more">
            {#if items.length > 0}
              <DropdownMenu {items} label={copyConsole.people.actions(person.name)}>
                {#snippet trigger(props)}
                  <IconButton icon="ellipsis" label={copyConsole.people.actions(person.name)} triggerProps={props} />
                {/snippet}
              </DropdownMenu>
            {/if}
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<AddPerson bind:open={adding} onAdded={load} />

{#if dialog}
  <AlertDialog
    bind:open={dialogOpen}
    title={dialog.title}
    description={dialog.body}
    cancelLabel={copy.action.cancel}
    confirmLabel={dialog.confirm}
    confirmVariant={dialog.variant}
    pending={authority.state === 'signing'}
    status={authority.state === 'signing' ? copyConsole.passkey.confirming : undefined}
    error={authority.error ?? undefined}
    onConfirm={confirm}
  >
    <p class="quiet">{copyConsole.people.passkeyNote}</p>
  </AlertDialog>
{/if}

<style>
  .screen {
    display: grid;
    gap: var(--s-5);
    max-width: var(--content-max);
    padding: var(--s-6) var(--content-pad);
  }
  .toolbar {
    display: flex;
    justify-content: flex-end;
  }
  .ask {
    max-width: var(--measure);
    color: var(--ink-2);
  }
  .failed {
    display: grid;
    justify-items: start;
    gap: var(--s-3);
  }
  .problem {
    color: var(--danger);
  }
  .list {
    display: grid;
  }
  .person {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr) var(--control);
    align-items: center;
    gap: var(--s-4);
    min-height: var(--row);
    padding: var(--s-2) var(--s-3);
    border-bottom: var(--hairline) solid var(--line);
  }
  .who,
  .devices {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1) var(--s-3);
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .you,
  .quiet {
    color: var(--ink-3);
  }
  .role {
    display: inline-flex;
    align-items: center;
    height: var(--chip);
    padding: 0 var(--s-2);
    border-radius: var(--r-1);
    font: var(--t-small);
    color: var(--ink-1);
    background: var(--sunken);
  }
  .role.member {
    background: transparent;
    border: var(--hairline) solid var(--line-strong);
  }
  .waiting {
    background: var(--sunken);
  }
  .note {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-1);
  }
  .device {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-2);
  }
  @media (width < 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
    .person {
      grid-template-columns: minmax(0, 1fr) var(--control);
      grid-template-areas: 'who more' 'devices devices';
      padding: var(--s-3) 0;
    }
    .who {
      grid-area: who;
    }
    .devices {
      grid-area: devices;
    }
    .more {
      grid-area: more;
    }
  }
</style>
