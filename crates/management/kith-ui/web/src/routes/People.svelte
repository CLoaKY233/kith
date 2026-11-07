<!--
@component
People (docs/plans/console.md): everyone in the network with their role and devices.
Owners and admins add people (a one-time link), and change roles or remove people through
an AlertDialog that says what changes, then the person's passkey confirms it. Members see
the same list read-only, with who to ask instead of controls.
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
  import { addableRoles, canRemove, roleChoices, runsNetwork, sortPeople } from '../lib/people';
  import type { AuthorityChange, Person } from '../lib/types';
  import AlertDialog from '../lib/ui/AlertDialog.svelte';
  import Button from '../lib/ui/Button.svelte';
  import DropdownMenu, { type MenuItem } from '../lib/ui/DropdownMenu.svelte';
  import IconButton from '../lib/ui/IconButton.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import { networkName } from '../frame/net';
  import AddPerson from '../sheets/AddPerson.svelte';

  let { net }: { net: string } = $props();

  let people = $state.raw<Person[] | null>(null);
  let loadError = $state<string | null>(null);
  let adding = $state(false);

  type Pending = { readonly person: Person; readonly change: AuthorityChange };
  let changing = $state.raw<Pending | null>(null);
  let dialogOpen = $state(false);
  const authority = new AuthorityPrompt();

  const viewer = $derived(me());
  const sorted = $derived(people ? sortPeople(people) : []);
  const admins = $derived(sorted.filter((p) => runsNetwork(p.role) && !p.pending).map((p) => p.name));
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

  onMount(() => {
    void load();
    // A device joining or leaving changes someone's list.
    return onEntry((entry) => {
      if (entry.kind === 'device_joined' || entry.kind === 'device_removed') void load();
    });
  });

  /** A device's state from the live store when it has one, else the list's. */
  function online(node: string, listed: boolean): boolean {
    if (node === live.status?.node) return true;
    return live.peers[net]?.[node]?.online ?? listed;
  }

  function ask(person: Person, change: AuthorityChange): void {
    changing = { person, change };
    authority.prepare(change);
    dialogOpen = true;
  }

  function menu(person: Person): MenuItem[] {
    const items: MenuItem[] = roleChoices(viewer, person).map((role) => ({
      label: role === 'admin' ? copyConsole.people.makeAdmin : copyConsole.people.makeMember,
      onSelect: () => ask(person, { kind: 'set_role', person: person.id, role }),
    }));
    if (canRemove(viewer, person)) {
      if (items.length > 0) items.push({ separator: true });
      items.push({
        label: copyConsole.people.remove,
        tone: 'danger',
        onSelect: () => ask(person, { kind: 'remove_person', person: person.id }),
      });
    }
    return items;
  }

  const dialog = $derived.by(() => {
    if (!changing) return null;
    const { person, change } = changing;
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
      change.kind === 'set_role' ? copyConsole.people.roleChanged(person.name, change.role) : copyConsole.people.removed(person.name),
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
        <li class="person">
          <div class="who">
            <span class="name">{person.name}</span>
            {#if person.id === viewer?.id}<span class="you">{copyConsole.people.you}</span>{/if}
            <span class="role {person.role}">{copy.account.role[person.role]}</span>
          </div>
          <div class="devices">
            {#if person.pending}
              <span class="quiet">{copyConsole.people.pending}</span>
            {:else if person.devices.length === 0}
              <span class="quiet">{copyConsole.people.noDevices}</span>
            {:else}
              {#each person.devices as device (device.node)}
                {@const on = online(device.node, device.online)}
                <span class="device">
                  <StatusDot state={on ? 'online' : 'offline'} />
                  {copyConsole.people.device(device.name, on)}
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
