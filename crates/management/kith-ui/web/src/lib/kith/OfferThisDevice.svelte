<!--
@component
OfferThisDevice: whether this device offers itself to its networks as an exit and as a
relay (`kith server`), through `GET` and `POST /api/server`. Each switch opens an
AlertDialog that says what changes before anything does: an exit sees where its users'
traffic goes, a relay carries encrypted traffic, both use this device's bandwidth, and
turning one off moves the devices that used it. Only an OS user who may change kith here
(`status.operator`) gets the switches; anyone else reads the current state.
Used by This device and Exits.
Props:
  none
-->
<script lang="ts">
  import { consoleMode } from '../account.svelte';
  import { untrack } from 'svelte';
  import { announce } from '../announce';
  import { errorText } from '../api';
  import { copy, copyScreens } from '../copy';
  import { actions, live } from '../live/store.svelte';
  import type { ActionHandle, ServerMode } from '../types';
  import type { Outcome } from '../types.gen';
  import AlertDialog from '../ui/AlertDialog.svelte';
  import Switch from '../ui/Switch.svelte';
  import { toast } from '../ui/Toast.svelte';

  $effect(() => {
    untrack(() => actions.loadServer());
  });

  const mode = $derived<ServerMode>(
    live.server ?? { exit: live.status?.exit ?? false, relay: live.status?.relay ?? false },
  );
  const operator = $derived(live.status?.operator ?? false);
  const device = $derived(live.status?.memberships[0]?.name ?? live.status?.host.name ?? '');
  /** On a public Console this device is the server, named outright. */
  const remote = $derived(consoleMode() === 'public');
  const frozen = $derived(live.conn === 'stale' || live.conn === 'daemon_down');

  // The switches follow the server; a cancelled change puts them back.
  let exitOn = $state(false);
  let relayOn = $state(false);
  function sync(): void {
    exitOn = mode.exit;
    relayOn = mode.relay;
  }
  $effect(() => {
    void mode;
    untrack(sync);
  });

  type Change = { readonly what: 'exit' | 'relay'; readonly on: boolean };
  let change = $state<Change | null>(null);
  let open = $state(false);
  let saving = $state<ActionHandle<Outcome> | null>(null);
  const pending = $derived(saving?.state === 'pending');
  const failed = $derived(
    saving?.state === 'failed' && saving.error ? errorText(saving.error) : undefined,
  );

  function ask(what: 'exit' | 'relay', on: boolean): void {
    if (frozen) return;
    change = { what, on };
    saving = null;
    open = true;
  }

  async function apply(): Promise<void> {
    const c = change;
    if (!c || pending) return;
    const next: ServerMode = { ...mode, [c.what]: c.on };
    const handle = actions.setServer(next);
    saving = handle;
    await handle.done;
    if (handle.state === 'failed') return;
    open = false;
    const done =
      c.what === 'exit'
        ? c.on
          ? copy.server.exitOn(device)
          : copy.server.exitOff(device)
        : c.on
          ? copy.server.relayOn(device)
          : copy.server.relayOff(device);
    announce(done);
    toast.done(done);
  }

  const dialog = $derived.by(() => {
    if (!change) return null;
    const s = copy.server;
    if (change.what === 'exit') {
      return change.on
        ? { title: s.exitOnTitle(device), body: s.exitOnBody(device), more: s.exitOnThen, submit: s.exitOnSubmit(device) }
        : { title: s.exitOffTitle(device), body: s.exitOffBody, more: null, submit: s.exitOffSubmit(device) };
    }
    return change.on
      ? { title: s.relayOnTitle(device), body: s.relayOnBody(device), more: s.cost, submit: s.relayOnSubmit(device) }
      : { title: s.relayOffTitle(device), body: s.relayOffBody, more: null, submit: s.relayOffSubmit(device) };
  });

  const headingId = $props.id();
</script>

<section class="offer" aria-labelledby={headingId}>
  <h2 class="t-heading" id={headingId}>{remote ? copy.server.serverHeading(device) : copy.server.heading}</h2>
  {#if operator}
    <Switch
      bind:checked={exitOn}
      disabled={frozen}
      label={remote ? copy.server.serverExit(device) : copy.server.exit}
      helper={copy.server.exitHelp}
      onCheckedChange={(on) => ask('exit', on)}
    />
    <Switch
      bind:checked={relayOn}
      disabled={frozen}
      label={remote ? copy.server.serverRelay(device) : copy.server.relay}
      helper={copy.server.relayHelp}
      onCheckedChange={(on) => ask('relay', on)}
    />
    <p class="quiet">{frozen ? copy.conn.staleActions : copy.server.cost}</p>
  {:else}
    <dl class="facts">
      <div><dt>{copy.server.exit}</dt><dd>{mode.exit ? copyScreens.settings.on : copyScreens.settings.off}</dd></div>
      <div><dt>{copy.server.relay}</dt><dd>{mode.relay ? copyScreens.settings.on : copyScreens.settings.off}</dd></div>
    </dl>
    <p class="quiet">{copy.server.notOperator}</p>
  {/if}
</section>

{#if dialog}
  <AlertDialog
    bind:open
    title={dialog.title}
    description={dialog.body}
    cancelLabel={copy.action.cancel}
    confirmLabel={dialog.submit}
    confirmVariant={change?.on ? 'primary' : 'danger'}
    {pending}
    error={failed}
    onConfirm={() => void apply()}
    onOpenChange={(next) => {
      if (!next) sync();
    }}
  >
    {#if dialog.more}<p>{dialog.more}</p>{/if}
  </AlertDialog>
{/if}

<style>
  .offer {
    display: grid;
    gap: var(--s-3);
    max-width: calc(var(--measure) + var(--s-8));
  }
  .facts {
    display: grid;
    gap: var(--s-2);
  }
  .facts div {
    display: flex;
    justify-content: space-between;
    gap: var(--s-4);
  }
  dd {
    color: var(--ink-2);
  }
  .quiet {
    font: var(--t-small);
    color: var(--ink-3);
    max-width: var(--measure);
  }
</style>
