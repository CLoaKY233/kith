<!--
@component
Settings (spec §6.8). Appearance (theme, compact rows on a fine pointer, keyboard
shortcuts) and the notification switch are this viewer's, kept in localStorage. Privacy
shows whether this device shares link reports. Network lists the admins and, for admins,
the open invites (expiry, uses left) with Revoke, then the relays: each with the device
that runs it, Remove (tier 0, with Undo) and an inline `Add relay` row; members get
`Ask an admin…` there. The danger zone holds `Leave home…`, tier 3: the network's
name typed, then confirmation on this computer.
Props:
  - net: string, the network as the route names it.
  - query?: URLSearchParams. `?relay=new` (the palette's `Add relay`) focuses the new relay.
-->
<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { go, setQuery } from '../router';
  import { announce } from '../lib/announce';
  import { errorText } from '../lib/api';
  import { copy, copyScreens, DOT } from '../lib/copy';
  import AskAdminSheet from '../lib/kith/AskAdminSheet.svelte';
  import { clock, weekdayClock } from '../lib/format';
  import { devicesByName, findNetwork, membershipOf } from '../lib/kith/network';
  import { actions, live, notifications } from '../lib/live/store.svelte';
  import { prefs } from '../frame/prefs.svelte';
  import { UNDO_MS } from '../lib/live/timing';
  import type { ActionHandle, ApiFailure, OpenInvite, Relay } from '../lib/types';
  import type { Outcome } from '../lib/types.gen';
  import Combobox from '../lib/ui/Combobox.svelte';
  import AlertDialog from '../lib/ui/AlertDialog.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import { toast } from '../lib/ui/Toast.svelte';

  let { net, query }: { net: string; query?: URLSearchParams } = $props();

  const network = $derived(findNetwork(live.status, net));
  const self = $derived(network ? membershipOf(live.status, network.id) : null);
  const admin = $derived(self?.role === 'admin');
  const peers = $derived(network ? devicesByName(live.peers, network.id) : []);
  /** §8.5: nothing is changed or queued while kith is away. */
  const frozen = $derived(live.conn === 'stale' || live.conn === 'daemon_down');

  function failureText(failure: ApiFailure | null): string | null {
    if (!failure) return null;
    switch (failure.code) {
      case 'permission_denied':
      case 'forbidden':
        return copy.error.permissionDenied;
      case 'os_auth_denied':
        return copy.error.osAuthDenied;
      case 'daemon_down':
        return copy.conn.staleActions;
      case 'unauthorized':
      case 'session_used':
      case 'session_expired':
        return copy.conn.unauth;
      default:
        return copy.error.internal(failure.message);
    }
  }

  // Appearance.
  const themeOptions = (['system', 'light', 'dark'] as const).map((value) => ({
    value,
    label: copy.settings.themes[value],
  }));
  function pickTheme(value: string): void {
    const theme = themeOptions.find((o) => o.value === value)?.value;
    if (theme) prefs.theme = theme;
  }
  let compact = $state(prefs.density === 'compact');
  let shortcuts = $state(prefs.shortcuts);
  let finePointer = $state(false);

  // Invites.
  let loadInvites = $state<ActionHandle<OpenInvite[]> | null>(null);
  const now = $derived(live.lastTickMs || live.now());
  const invites = $derived(
    network ? (live.invites[network.id] ?? []).filter((i) => i.expires_at_ms > now) : [],
  );
  let revoking = $state<Record<string, ActionHandle<Outcome>>>({});

  /** `15:42` today, `Tue 15:42` later in the week (invites last a week at most). */
  function expiry(ms: number): string {
    const sameDay = new Date(ms).toDateString() === new Date(now).toDateString();
    return sameDay ? clock(ms) : weekdayClock(ms);
  }

  function inviteStatus(invite: OpenInvite): string {
    return copyScreens.settings.inviteRow(expiry(invite.expires_at_ms), invite.uses_left);
  }

  async function revoke(invite: OpenInvite): Promise<void> {
    if (!network || frozen) return;
    const id = network.id;
    const handle = actions.revokeInvite(id, invite.id);
    revoking = { ...revoking, [invite.id]: handle };
    const done = await handle.done;
    if (done.state === 'accepted' || done.state === 'seen') toast.done(copy.invite.revoked);
  }

  // Relays.
  let loadRelays = $state<ActionHandle<Relay[]> | null>(null);
  const settled = (handle: ActionHandle<unknown> | null): boolean =>
    handle !== null && handle.state !== 'idle' && handle.state !== 'pending';
  const listsSettled = $derived((!admin || settled(loadInvites)) && settled(loadRelays));
  const relays = $derived(
    network ? [...(live.relays[network.id] ?? [])].sort((a, b) => a.name.localeCompare(b.name)) : [],
  );
  const selfNode = $derived(live.status?.node ?? null);
  function runner(node: string | null): string {
    if (node === null || !network) return copy.relays.outside;
    if (node === selfNode) return copy.relays.runBy(self?.name ?? node);
    return copy.relays.runBy(live.peers[network.id]?.[node]?.name ?? node.slice(0, 8));
  }
  let relayName = $state('');
  let relayAddress = $state('');
  let relayNode = $state('');
  let relayNameError = $state<string | null>(null);
  let relayAddressError = $state<string | null>(null);
  let relayAdding = $state<ActionHandle<Outcome> | null>(null);
  let relayRemoving = $state<Record<string, ActionHandle<Outcome>>>({});
  let relayNameRef = $state<HTMLInputElement | null>(null);
  const runnerOptions = $derived([
    { value: '', label: copy.relays.runsOnNone },
    ...(self && selfNode ? [{ value: selfNode, label: self.name }] : []),
    ...peers.map((p) => ({ value: p.node, label: p.name })),
  ]);

  async function addRelay(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!network || frozen || relayAdding?.state === 'pending') return;
    const name = relayName.trim().toLowerCase();
    const address = relayAddress.trim();
    relayNameError = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/.test(name)
      ? relays.some((r) => r.name === name)
        ? copy.relays.taken(name)
        : null
      : name === ''
        ? copy.error.nameEmpty
        : copy.error.nameChars;
    relayAddressError = /^(\[[0-9a-fA-F:.]+\]|[^\s:/]+):\d{1,5}$/.test(address) &&
      Number(address.slice(address.lastIndexOf(':') + 1)) <= 65535
      ? null
      : copy.relays.addressInvalid;
    const problem = relayNameError ?? relayAddressError;
    if (problem) {
      announce(problem);
      return;
    }
    const handle = actions.addRelay(network.id, name, address, relayNode === '' ? null : relayNode);
    relayAdding = handle;
    await handle.done;
    if (handle.state === 'failed' && handle.error) {
      announce(errorText(handle.error));
      return;
    }
    if (handle.state === 'conflict') return;
    relayName = '';
    relayAddress = '';
    relayNode = '';
    toast.done(copy.relays.added(name));
  }

  async function removeRelay(relay: Relay): Promise<void> {
    if (!network || frozen) return;
    const id = network.id;
    const handle = actions.removeRelay(id, relay.name);
    relayRemoving = { ...relayRemoving, [relay.name]: handle };
    await handle.done;
    if (handle.state === 'failed' && handle.error) {
      toast.failed(errorText(handle.error));
      return;
    }
    toast.done(copy.relays.removed(relay.name), {
      undoMs: UNDO_MS,
      undo: () => void actions.addRelay(id, relay.name, relay.address, relay.node),
    });
  }

  let askRelay = $state(false);

  // `?relay=new` from the palette: focus the new relay's name, then drop the parameter.
  $effect(() => {
    if (query?.get('relay') !== 'new' || !admin || !relayNameRef) return;
    untrack(() => {
      setQuery({ relay: null }, { replace: true });
      void tick().then(() => relayNameRef?.focus());
    });
  });

  // Leave (tier 3, §13).
  let leaving = $state(false);
  let typed = $state('');
  let confirmInput: HTMLInputElement | null = $state(null);
  let leave = $state<ActionHandle<Outcome> | null>(null);
  const lastAdmin = $derived(admin && (self?.admins.length ?? 0) <= 1);
  const leaveError = $derived(leave?.state === 'failed' ? failureText(leave.error) : null);
  const waitingForOs = $derived(
    leave?.state === 'pending' || leave?.state === 'awaiting_os_auth',
  );

  function openLeave(): void {
    if (frozen) return;
    typed = '';
    leave = null;
    leaving = true;
  }

  async function confirmLeave(): Promise<void> {
    if (!network || typed !== network.name || frozen || waitingForOs) return;
    const name = network.name;
    const handle = actions.leave(network.id);
    leave = handle;
    const done = await handle.done;
    if (done.state === 'accepted') {
      leaving = false;
      toast.done(copyScreens.settings.left(name));
      go('/');
    }
  }

  onMount(() => {
    const fine = window.matchMedia('(pointer: fine)');
    const update = (): void => {
      finePointer = fine.matches;
    };
    update();
    fine.addEventListener('change', update);
    return () => fine.removeEventListener('change', update);
  });

  $effect(() => {
    const id = network?.id;
    if (id && admin) loadInvites = untrack(() => actions.loadInvites(id));
  });
  $effect(() => {
    const id = network?.id;
    if (id) loadRelays = untrack(() => actions.loadRelays(id));
  });
</script>

<section class="screen">

  <section class="group" aria-labelledby="settings-appearance">
    <h2 class="t-heading" id="settings-appearance">{copy.settings.appearance}</h2>
    <div class="setting">
      <span>{copy.settings.theme}</span>
      <Segmented
        value={prefs.theme}
        options={themeOptions}
        label={copy.settings.theme}
        onValueChange={pickTheme}
      />
    </div>
    {#if finePointer}
      <Switch
        bind:checked={compact}
        label={copy.settings.compact}
        onCheckedChange={(on) => prefs.density = on ? 'compact' : 'comfortable'}
      />
    {/if}
    <Switch
      bind:checked={shortcuts}
      label={copy.settings.shortcuts}
      onCheckedChange={(on) => prefs.shortcuts = on}
    />
  </section>

  <section class="group" aria-labelledby="settings-notifications">
    <h2 class="t-heading" id="settings-notifications">{copy.settings.notifications}</h2>
    {#if notifications.supported}
      <Switch
        checked={notifications.on}
        label={copyScreens.settings.notifySwitch}
        helper={notifications.permission === 'denied'
          ? copyScreens.settings.notificationsBlocked
          : copy.activity.notifyCardBody}
        onCheckedChange={(on) => void notifications.setEnabled(on)}
      />
    {:else}
      <p class="quiet">{copyScreens.settings.notificationsUnsupported}</p>
    {/if}
  </section>

  {#if network && self}
    <section class="group" aria-labelledby="settings-privacy">
      <h2 class="t-heading" id="settings-privacy">{copy.settings.privacy}</h2>
      <div class="setting readonly">
        <div class="text">
          <span>{copy.settings.shareLinks(network.name)}</span>
          <p class="quiet">{copy.settings.shareLinksHelp}</p>
        </div>
        <span class="value">
          {self.share_links ? copyScreens.settings.on : copyScreens.settings.off}
        </span>
      </div>
    </section>

    <!-- Once the lists have loaded, so the sections below them never move (§11.9). -->
    {#if listsSettled}
      <section class="group" aria-labelledby="settings-network">
        <h2 class="t-heading" id="settings-network">{copy.settings.network}</h2>

        <h3 class="label">{copy.settings.admins}</h3>
        <ul class="list">
          {#each self.admins as name (name)}
            {@const peer = peers.find((p) => p.name === name)}
            <li class="item">
              <span class="name">{name}</span>
              {#if name === self.name}
                <Badge>{copy.device.you}</Badge>
              {:else if peer}
                <span class="state">
                  <StatusDot state={peer.online ? 'online' : 'offline'} />
                  {peer.online ? copy.status.online : copy.status.offline}
                </span>
              {/if}
            </li>
          {/each}
        </ul>

        {#if admin}
          <h3 class="label">{copy.settings.invites}</h3>
          {#if loadInvites?.state === 'failed'}
            <p class="error">{failureText(loadInvites.error)}</p>
          {:else if loadInvites?.state === 'accepted' && invites.length === 0}
            <p class="quiet">{copyScreens.settings.noInvites}</p>
          {:else if invites.length > 0}
            <ul class="list">
              {#each invites as invite (invite.id)}
                {@const handle = revoking[invite.id]}
                <li class="item invite">
                  <div class="text">
                    <span>{inviteStatus(invite)}</span>
                    <span class="quiet">{copyScreens.settings.inviteMade(expiry(invite.created_at_ms))}</span>
                    {#if handle?.state === 'failed'}
                      <p class="error">{failureText(handle.error)}</p>
                    {/if}
                  </div>
                  <Button
                    variant="secondary"
                    pending={handle?.state === 'pending'}
                    disabled={frozen}
                    aria-label={copyScreens.settings.revokeLabel(expiry(invite.expires_at_ms))}
                    onclick={() => void revoke(invite)}
                  >
                    {copy.settings.revoke}
                  </Button>
                </li>
              {/each}
            </ul>
          {/if}
        {/if}
        {#if frozen}<p class="quiet">{copy.conn.staleActions}</p>{/if}
      </section>

      <section class="group" aria-labelledby="settings-relays">
        <h2 class="t-heading" id="settings-relays">{copy.relays.heading}</h2>
        <p class="quiet">{copy.relays.intro}</p>
        {#if loadRelays?.state === 'failed'}
          <p class="error">{failureText(loadRelays.error)}</p>
        {:else if loadRelays?.state === 'accepted' && relays.length === 0}
          <p class="quiet">{copy.relays.none}</p>
        {/if}
        {#if relays.length > 0}
          <ul class="list">
            {#each relays as relay (relay.name)}
              {@const handle = relayRemoving[relay.name]}
              <li class="item invite">
                <div class="text">
                  <span class="name">{relay.name}</span>
                  <span class="quiet"><code class="mono">{relay.address}</code>{DOT}{runner(relay.node)}</span>
                </div>
                {#if admin}
                  <Button
                    variant="secondary"
                    pending={handle?.state === 'pending'}
                    disabled={frozen}
                    aria-label={copy.relays.removeLabel(relay.name)}
                    onclick={() => void removeRelay(relay)}
                  >
                    {copy.relays.remove}
                  </Button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        {#if admin}
          <form class="relay-form" novalidate onsubmit={(e) => void addRelay(e)}>
            <Field label={copy.relays.name} error={relayNameError}>
              {#snippet children(control)}
                <Input
                  {...control}
                  kind="name"
                  bind:value={relayName}
                  bind:ref={relayNameRef}
                  oninput={() => (relayNameError = null)}
                />
              {/snippet}
            </Field>
            <Field label={copy.relays.address} helper={copy.relays.addressHelp} error={relayAddressError}>
              {#snippet children(control)}
                <Input
                  {...control}
                  kind="mono"
                  bind:value={relayAddress}
                  autocapitalize="none"
                  autocorrect="off"
                  spellcheck={false}
                  oninput={() => (relayAddressError = null)}
                />
              {/snippet}
            </Field>
            <div class="pick">
              <span class="field-label" aria-hidden="true">{copy.relays.runsOn}</span>
              <Combobox
                value={relayNode}
                options={runnerOptions}
                label={copy.relays.runsOn}
                empty={copy.access.noDevices}
                onValueChange={(v) => (relayNode = v)}
              />
            </div>
            {#if relayAdding?.state === 'failed' && relayAdding.error}
              <p class="error">{errorText(relayAdding.error)}</p>
            {/if}
            <div>
              <Button
                type="submit"
                pending={relayAdding?.state === 'pending'}
                pendingLabel={copy.action.signing}
                disabled={frozen}
              >
                {copy.relays.add}
              </Button>
            </div>
          </form>
        {:else}
          <div><Button onclick={() => (askRelay = true)}>{copy.relays.ask}</Button></div>
          <AskAdminSheet
            bind:open={askRelay}
            net={network.id}
            request={copy.askAdmin.request.addRelay(network.name)}
          />
        {/if}
      </section>

      <section class="group" aria-labelledby="settings-danger">
        <h2 class="t-heading" id="settings-danger">{copy.settings.danger}</h2>
        {#if lastAdmin}
          <p class="quiet">{copy.remove.lastAdmin(self.name)}</p>
        {:else}
          <div>
            <Button variant="danger-quiet" disabled={frozen} onclick={openLeave}>
              {copy.settings.leave(network.name)}
            </Button>
          </div>
          {#if frozen}<p class="quiet">{copy.conn.staleActions}</p>{/if}
        {/if}
      </section>
    {/if}

    <AlertDialog
      bind:open={leaving}
      title={copyScreens.settings.leaveTitle(network.name)}
      description={copyScreens.settings.leaveLoses(network.name)}
      cancelLabel={copy.action.cancel}
      confirmLabel={copyScreens.settings.leaveSubmit(network.name)}
      confirmDisabled={typed !== network.name || frozen}
      pending={waitingForOs}
      status={waitingForOs ? copy.tier3.waiting : undefined}
      initialFocus={confirmInput}
      onConfirm={() => void confirmLeave()}
    >
      <p>{copy.remove.comeBack}</p>
      <!-- Enter submits only once the name matches; confirmLeave checks it (tier 3). -->
      <form
        onsubmit={(event) => {
          event.preventDefault();
          void confirmLeave();
        }}
      >
        <Field label={copy.settings.typeToConfirm(network.name)} error={leaveError}>
          {#snippet children(control)}
            <Input kind="name" bind:value={typed} bind:ref={confirmInput} {...control} />
          {/snippet}
        </Field>
      </form>
    </AlertDialog>
  {/if}
</section>

<style>
  .screen {
    display: grid;
    gap: var(--s-7);
    max-width: var(--content-max);
    margin: 0;
    padding: var(--s-6) var(--content-pad);
  }
  .group {
    display: grid;
    gap: var(--s-2);
    max-width: calc(var(--measure) + var(--s-8));
  }
  .setting {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
    min-height: var(--row);
  }
  .setting.readonly {
    flex-wrap: nowrap;
  }
  .text {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  .value {
    color: var(--ink-2);
  }
  .quiet {
    font: var(--t-small);
    color: var(--ink-3);
    max-width: var(--measure);
  }
  .error {
    font: var(--t-small);
    color: var(--danger);
  }
  .label {
    padding-top: var(--s-3);
    color: var(--ink-3);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    font-variant-caps: all-small-caps;
  }
  .list {
    display: grid;
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: var(--row);
  }
  .item.invite {
    justify-content: space-between;
    padding: var(--s-2) 0;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-2);
    font: var(--t-small);
  }
  .mono {
    font: var(--t-mono);
  }
  .relay-form {
    display: grid;
    gap: var(--s-3);
    padding-top: var(--s-3);
  }
  .pick {
    display: grid;
    justify-items: start;
    gap: var(--s-1);
  }
  .field-label {
    font-weight: 600;
  }

  @media (width < 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
  }
</style>
