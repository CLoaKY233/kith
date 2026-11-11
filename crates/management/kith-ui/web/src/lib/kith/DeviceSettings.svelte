<!--
@component
DeviceSettings: where one device sends its internet traffic, written into the network so the
device applies it itself: an exit (or its own connection), `Lock` (only admins and its owner
may change it) and `Pause`. One Save sends all three.

The line under the controls is honest: `Waiting for tv…` until the exit reports the device
among its users, `Set · applies when tv is online` while it is offline, then `Using vps`.
Admins change any device; a member only their own (the people list says which). Everyone
else sees the line alone, and nothing when nothing is set.
Props:
  - net: string, network id.
  - node: string, the device id.
  - name: string, the device's name in the network.
  - online: boolean.
  - isSelf: boolean, the Console's own device (its settings come with the status).
  - stale?: boolean, values are last known (§8.5): Save is aria-disabled with the reason.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { me } from '../account.svelte';
  import { copy } from '../copy';
  import { actions, live } from '../live/store.svelte';
  import { runsNetwork } from '../people';
  import type { ActionHandle } from '../types';
  import type { DeviceSettingsInfo, Outcome, PersonView } from '../types.gen';
  import Button from '../ui/Button.svelte';
  import Combobox from '../ui/Combobox.svelte';
  import Switch from '../ui/Switch.svelte';
  import { appliedOf, choosable, owns, same } from './deviceSettings';
  import { errorText } from '../api';

  const EXITS_REFRESH_MS = 4000;

  let {
    net,
    node,
    name,
    online,
    isSelf,
    stale = false,
  }: {
    net: string;
    node: string;
    name: string;
    online: boolean;
    isSelf: boolean;
    stale?: boolean;
  } = $props();

  // Not empty: the field drops an empty choice. Node ids are hex, so this can't clash.
  const NONE = 'own';
  const network = $derived(live.status?.networks.find((n) => n.id === net) ?? null);
  const saved = $derived<DeviceSettingsInfo | null>(
    isSelf ? (network?.my_settings ?? null) : (live.peers[net]?.[node]?.settings ?? null),
  );
  const exits = $derived(live.exits[net] ?? null);

  // A member changes their own devices only, and the people list says which are theirs.
  let people = $state.raw<PersonView[] | null>(null);
  let asked = false;
  const privileged = $derived((network?.admin ?? false) || runsNetwork(me()?.role));
  $effect(() => {
    if (privileged || me() === null || asked) return;
    asked = true;
    void untrack(() => actions.loadPeople().done).then((h) => {
      if (h.state === 'accepted' || h.state === 'seen') people = h.result ?? null;
    });
  });
  $effect(() => {
    if (exits === null) untrack(() => actions.loadExits(net));
  });
  // Who uses an exit comes from the exits list, so keep it fresh while an exit is chosen:
  // "Waiting…" turns into "Using vps" without a reload.
  const watching = $derived(saved !== null && saved.exit !== null && !saved.paused);
  $effect(() => {
    if (!watching) return;
    const timer = setInterval(() => untrack(() => actions.loadExits(net)), EXITS_REFRESH_MS);
    return () => clearInterval(timer);
  });
  const mayChange = $derived(privileged || owns(people, node));

  const base: DeviceSettingsInfo = { exit: null, exit_locked: false, paused: false };
  let draft = $state<DeviceSettingsInfo | null>(null);
  const shown = $derived(draft ?? saved ?? base);
  const dirty = $derived(draft !== null && !same(draft, saved ?? base));
  $effect(() => {
    if (draft !== null && same(draft, saved ?? base)) draft = null;
  });

  const options = $derived([
    { value: NONE, label: copy.deviceSettings.own },
    ...choosable(exits ?? [], node, shown.exit).map((e) => ({ value: e.node, label: e.name })),
  ]);

  function edit(change: Partial<DeviceSettingsInfo>): void {
    draft = { ...shown, ...change };
  }

  let handle = $state<ActionHandle<Outcome> | null>(null);
  const busy = $derived(handle?.state === 'pending' || handle?.state === 'awaiting_os_auth');

  function save(): void {
    if (draft === null || stale) return;
    const next = draft;
    handle = actions.setDeviceSettings(net, node, next);
  }

  const note = $derived.by(() => {
    if (handle?.state === 'pending') return { text: copy.action.signing, failed: false };
    if (handle?.state === 'awaiting_os_auth') return { text: copy.action.awaitingOsAuth, failed: false };
    if ((handle?.state === 'failed' || handle?.state === 'conflict') && handle.error) {
      return { text: errorText(handle.error), failed: true };
    }
    return null;
  });

  const status = $derived.by(() => {
    if (saved === null || dirty) return null;
    const applied = appliedOf({
      settings: saved,
      name,
      online,
      exits: exits ?? [],
      observer: live.status?.node ?? null,
      ...(isSelf ? { usedNow: live.status?.exit_node ?? null, network: network?.name ?? '' } : {}),
    });
    switch (applied.kind) {
      case 'using':
        return copy.deviceSettings.using(applied.exit);
      case 'own':
        return copy.deviceSettings.usingOwn;
      case 'paused':
        return copy.deviceSettings.paused;
      case 'offline':
        return copy.deviceSettings.offline(name);
      case 'waiting':
        return copy.deviceSettings.waiting(name);
      case 'set':
        return copy.deviceSettings.set(name, applied.exit);
    }
  });
  const headingId = $props.id();
</script>

{#if mayChange || saved !== null}
  <section class="section" aria-labelledby={headingId}>
    <h3 class="t-heading" id={headingId}>{copy.deviceSettings.heading}</h3>
    {#if mayChange}
      <div class="controls">
        <div class="pick">
          <span class="label">{copy.deviceSettings.exit}</span>
          <Combobox
            value={shown.exit ?? NONE}
            {options}
            label={copy.deviceSettings.exit}
            empty={copy.deviceSettings.noMatch}
            onValueChange={(value) => edit({ exit: value === NONE ? null : value })}
          />
          {#if exits !== null && options.length === 1}
            <p class="helper">{copy.deviceSettings.noExits}</p>
          {/if}
        </div>
        <Switch
          checked={shown.exit_locked}
          label={copy.deviceSettings.lock}
          helper={copy.deviceSettings.lockHelper}
          disabled={stale}
          onCheckedChange={(on) => edit({ exit_locked: on })}
        />
        <Switch
          checked={shown.paused}
          label={copy.deviceSettings.pause}
          helper={copy.deviceSettings.pauseHelper}
          disabled={stale}
          onCheckedChange={(on) => edit({ paused: on })}
        />
      </div>
      {#if dirty || stale}
        <div class="actions">
          {#if dirty}
            <Button variant="primary" pending={busy} disabled={stale} onclick={save}>
              {copy.deviceSettings.save}
            </Button>
          {/if}
          {#if stale}<p class="helper">{copy.conn.staleActions}</p>{/if}
        </div>
      {/if}
    {/if}
    {#if note}
      <p class="status" class:failed={note.failed} role="status">{note.text}</p>
    {:else if status}
      <p class="status" role="status">{status}</p>
    {/if}
  </section>
{/if}

<style>
  .section {
    display: grid;
    align-content: start;
    gap: var(--s-3);
  }
  .controls {
    display: grid;
    gap: var(--s-2);
  }
  .pick {
    display: grid;
    gap: var(--s-1);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
  }
  .label {
    font: var(--t-small);
    color: var(--ink-2);
  }
  .helper,
  .status {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .failed {
    color: var(--danger);
  }
</style>
