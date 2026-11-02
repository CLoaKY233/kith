<!--
@component
Settings (spec §6.8). Appearance (theme, compact rows on a fine pointer, keyboard
shortcuts) and the notification switch are this viewer's, kept in localStorage. Privacy
shows whether this device shares link reports. Network lists the admins and, for admins,
the open invites with Revoke. The danger zone holds `Leave home…`, tier 3: the network's
name typed, then confirmation on this computer.
Props:
  - net: string, the network as the route names it.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { go } from '../router';
  import { copy, copyScreens, DOT } from '../lib/copy';
  import { clock, weekdayClock } from '../lib/format';
  import { devicesByName, findNetwork, membershipOf } from '../lib/kith/network';
  import { actions, live, notifications } from '../lib/live/store.svelte';
  import { prefs, setPref } from '../lib/prefs.svelte';
  import type { ActionHandle, ApiFailure } from '../lib/types';
  import type { Invite, Outcome } from '../lib/types.gen';
  import AlertDialog from '../lib/ui/AlertDialog.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import { toast } from '../lib/ui/Toast.svelte';

  let { net }: { net: string } = $props();

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
      case 'not_implemented':
        return copy.error.notImplemented;
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
    if (theme) setPref('theme', theme);
  }
  let compact = $state(prefs.density === 'compact');
  let shortcuts = $state(prefs.shortcuts);
  let finePointer = $state(false);

  // Invites.
  let loadInvites = $state<ActionHandle<Invite[]> | null>(null);
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

  function inviteStatus(invite: Invite): string {
    if (invite.used_by.length === 0) return copy.invite.notUsed(expiry(invite.expires_at_ms));
    const used = copyScreens.settings.inviteUsedBy(invite.used_by);
    if (invite.uses === null) return used;
    const left = Math.max(0, invite.uses - invite.used_by.length);
    return `${used}${DOT}${copyScreens.settings.inviteUsesLeft(left)}`;
  }

  async function revoke(invite: Invite): Promise<void> {
    if (!network || frozen) return;
    const id = network.id;
    const handle = actions.revokeInvite(id, invite.id);
    revoking = { ...revoking, [invite.id]: handle };
    const done = await handle.done;
    if (done.state === 'accepted') {
      toast.done(copy.invite.revoked);
      loadInvites = actions.loadInvites(id);
    }
  }

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
</script>

<section class="screen">
  <h1 class="t-title">{copy.nav.settings}</h1>

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
        onCheckedChange={(on) => setPref('density', on ? 'compact' : 'comfortable')}
      />
    {/if}
    <Switch
      bind:checked={shortcuts}
      label={copy.settings.shortcuts}
      onCheckedChange={(on) => setPref('shortcuts', on)}
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
      <div class="setting fixed">
        <div class="text">
          <span>{copy.settings.shareLinks(network.name)}</span>
          <p class="quiet">{copy.settings.shareLinksHelp}</p>
        </div>
        <span class="value">
          {self.share_links ? copyScreens.settings.on : copyScreens.settings.off}
        </span>
      </div>
    </section>

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
                  <code class="link">{invite.link}</code>
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
    margin: 0 auto;
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
  .setting.fixed {
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
  .link {
    overflow: hidden;
    color: var(--ink-3);
    font: var(--t-mono);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (width < 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
  }
</style>
