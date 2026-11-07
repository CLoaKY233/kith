<!--
@component
Exits (spec §6.7). This device's choice at the top (ExitPicker), then, for admins, every
exit on the network: who has access, who is using it now (the exit's own `in_use_by`) and
its live rate. Each row has an `Allowed` switch; one that may not be used reads `Not allowed
yet`. Switching either way is tier 2: an AlertDialog with the consequences, and the button
names the exit. A line says devices pick their exit on their own page, and links to Devices.
This device is `This device` in the lists, first. `Allow device…` (§6.7) waits for the API: an exit grant is network-wide today.
Below them: imported WireGuard networks (ImportedExits: use, stop using, remove),
`Import a WireGuard config…` (`?sheet=wireguard`), and this device offered as an exit or relay.
Props:
  - net: string, the network's id or name (as in the URL).
  - query?: URLSearchParams, for `?sheet=wireguard`.
-->
<script lang="ts">
  import { consoleMode } from '../lib/account.svelte';
  import { announce } from '../lib/announce';
  import { copy } from '../lib/copy';
  import ExitPicker from '../lib/kith/ExitPicker.svelte';
  import ImportedExits from '../lib/kith/ImportedExits.svelte';
  import OfferThisDevice from '../lib/kith/OfferThisDevice.svelte';
  import ImportWireguard from '../sheets/ImportWireguard.svelte';
  import Button from '../lib/ui/Button.svelte';
  import { closeOverlay, go, href, openOverlay } from '../router';
  import Rate from '../lib/kith/Rate.svelte';
  import { actions, live } from '../lib/live/store.svelte';
  import { BANNER_DELAY_MS } from '../lib/live/timing';
  import type { ActionHandle, DotState } from '../lib/types';
  import type { ExitView, NetworkSummary, Outcome } from '../lib/types.gen';
  import AlertDialog from '../lib/ui/AlertDialog.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';

  let { net, query }: { net: string; query?: URLSearchParams } = $props();

  const importing = $derived(query?.get('sheet') === 'wireguard');
  let pickerReady = $state(false);

  function findNetwork(list: readonly NetworkSummary[], ref: string): NetworkSummary | null {
    return (
      list.find((n) => n.id === ref) ??
      list.find((n) => n.name === ref) ??
      (ref === '' ? undefined : list.find((n) => n.id.startsWith(ref))) ??
      null
    );
  }

  const network = $derived(findNetwork(live.status?.networks ?? [], net));
  const netId = $derived(network?.id ?? null);
  const admin = $derived(network?.admin ?? false);
  const self = $derived(live.status?.node ?? null);
  const selfName = $derived(
    live.status?.memberships.find((m) => m.network === netId)?.name ?? null,
  );
  const peers = $derived(netId ? Object.values(live.peers[netId] ?? {}) : []);

  /** Device names with this device first, as `This device`. */
  function people(names: readonly string[]): string[] {
    if (selfName === null || !names.includes(selfName)) return [...names];
    return [copy.exits.selfFirst, ...names.filter((n) => n !== selfName)];
  }
  const rows = $derived(
    [...(netId ? (live.exits[netId] ?? []) : [])].sort((a, b) => a.name.localeCompare(b.name)),
  );

  // §8.5: after STALE plus BANNER_DELAY, nothing can change until kith is back.
  let frozen = $state(false);
  $effect(() => {
    const down = live.conn === 'stale' || live.conn === 'daemon_down' || live.conn === 'ui_gone';
    if (!down) {
      frozen = false;
      return;
    }
    const timer = setTimeout(() => (frozen = true), BANNER_DELAY_MS);
    return () => clearTimeout(timer);
  });

  function online(exit: ExitView): boolean {
    if (exit.node === self) return true;
    return (netId ? live.peers[netId]?.[exit.node]?.online : undefined) ?? false;
  }

  function dot(exit: ExitView): DotState {
    if (!online(exit)) return 'offline';
    return frozen ? 'stale' : 'online';
  }

  /**
   * Bytes per second through the exit: the links to it from the devices using it now.
   * Nobody using it reads `—`; other traffic to the device is not exit traffic.
   */
  function rate(exit: ExitView): number | null {
    if (exit.in_use_by.length === 0) return null;
    const users = new Set(
      exit.in_use_by.map((name) =>
        name === selfName ? self : (peers.find((p) => p.name === name)?.node ?? null),
      ),
    );
    let sum: number | null = null;
    for (const link of netId ? (live.links[netId] ?? []) : []) {
      if (link.to === exit.node && users.has(link.from) && link.rate !== null) {
        sum = (sum ?? 0) + link.rate;
      }
    }
    if (sum !== null || self === null || !users.has(self)) return sum;
    const own = netId ? live.numbers[`${netId}/${exit.node}`] : undefined;
    return own ? own.rx + own.tx : null;
  }

  // Tier 2 (§13): the dialog shows what changes; the row then shows the signing.
  type Change = { exit: ExitView; allowed: boolean };
  let change = $state<Change | null>(null);
  let changeOpen = $state(false);
  let grants = $state<Record<string, ActionHandle<Outcome>>>({});

  function confirm(exit: ExitView, allowed: boolean): void {
    change = { exit, allowed };
    changeOpen = true;
  }

  function failure(handle: ActionHandle<Outcome>, exit: string): string {
    if (handle.state === 'conflict' && handle.conflict) {
      return copy.error.conflict(handle.conflict.resolved_by);
    }
    switch (handle.error?.code) {
      case 'permission_denied':
        return copy.error.permissionDenied;
      case 'daemon_down':
        return copy.conn.staleActions;
      case 'not_found':
        return copy.error.notFound(exit);
      default:
        return copy.error.internal(handle.error?.message ?? '');
    }
  }

  function progress(handle: ActionHandle<Outcome>, exit: string): string {
    switch (handle.state) {
      case 'pending':
        return copy.action.signing;
      case 'awaiting_os_auth':
        return copy.action.awaitingOsAuth;
      case 'failed':
      case 'conflict':
        return failure(handle, exit);
      default:
        if (handle.seen) return copy.action.spreading(handle.seen.by, handle.seen.of);
        return handle.slow ? copy.action.savedWaiting : copy.action.savedHere;
    }
  }

  function apply(): void {
    const current = change;
    committed = true;
    changeOpen = false;
    if (current === null || netId === null) return;
    const id = netId;
    const handle = actions.grantExit(id, current.exit.node, current.allowed);
    grants = { ...grants, [current.exit.node]: handle };
    void handle.done.then((h) => {
      if (h.state === 'accepted') actions.loadExits(id);
      else announce(failure(h, current.exit.name));
    });
  }

  // A switch moves at once; when the dialog is cancelled it is built again from the truth.
  let committed = false;
  let resets = $state(0);
  $effect(() => {
    if (changeOpen) return;
    if (committed) committed = false;
    else if (change !== null) resets += 1;
  });

  const headingId = $props.id();
  const reasonId = `${headingId}-reason`;
</script>

<section class="screen">
  {#if network}
    <div class="sections">
      {#if consoleMode() !== 'public'}
        <section>
          <ExitPicker {net} bind:ready={pickerReady} />
        </section>
      {/if}

      <!-- Below the picker once it has its height, so nothing moves (§11.9). -->
      {#if pickerReady || consoleMode() === 'public'}
        <p class="t-small hint">
          {copy.exits.devicesPick}
          <a
            href={href({ name: 'devices', net: network.name })}
            onclick={(event) => {
              event.preventDefault();
              go(href({ name: 'devices', net: network.name }));
            }}>{copy.exits.devicesLink}</a
          >
        </p>
        {#if admin && rows.length > 0}
          <section class="network" aria-labelledby={headingId}>
            <h2 class="t-heading" id={headingId}>{copy.exits.onNetwork}</h2>
            <div class="table" role="table" aria-labelledby={headingId}>
              <div class="row head t-micro" role="row">
                <span role="columnheader">{copy.exits.exit}</span>
                <span role="columnheader">{copy.exits.access}</span>
                <span role="columnheader">{copy.exits.inUse}</span>
                <span role="columnheader" class="num">{copy.device.columns.rate}</span>
                <span role="columnheader" class="num">{copy.exits.allowedColumn}</span>
              </div>
              {#each rows as exit (exit.node)}
                {@const grant = grants[exit.node]}
                <div class="row" class:locked={!exit.allowed} role="row">
                  <span class="who" role="cell">
                    <span class="line">
                      <StatusDot state={dot(exit)} />
                      <span class="name">{exit.name}</span>
                      <span class="sr-only">{online(exit) ? copy.status.online : copy.status.offline}</span>
                    </span>
                    {#if grant}
                      <span
                        class="status"
                        class:failed={grant.state === 'failed' || grant.state === 'conflict'}
                      >
                        {progress(grant, exit.name)}
                      </span>
                    {/if}
                  </span>
                  <span class="access" role="cell">
                    <span class="cell-label">{copy.exits.access}</span>
                    {#if !exit.allowed}
                      <span class="quiet">{copy.exits.notAllowed}</span>
                    {:else}
                      <span>
                        {exit.access.length === 0
                          ? copy.exits.everyone
                          : copy.exits.list(people(exit.access))}
                      </span>
                    {/if}
                  </span>
                  <span class="use" role="cell">
                    <span class="cell-label">{copy.exits.inUse}</span>
                    <span class:quiet={exit.in_use_by.length === 0}>
                      {exit.in_use_by.length === 0
                        ? copy.exits.nobody
                        : copy.exits.list(people(exit.in_use_by))}
                    </span>
                  </span>
                  <span class="num" role="cell"><Rate value={rate(exit)} stale={frozen} /></span>
                  <span class="more" role="cell">
                    {#key `${exit.node}/${exit.allowed}/${resets}`}
                      <Switch
                        checked={exit.allowed}
                        label={copy.exits.toggleFor(exit.name)}
                        hideLabel
                        disabled={frozen}
                        onCheckedChange={(on) => confirm(exit, on)}
                      />
                    {/key}
                  </span>
                </div>
              {/each}
            </div>
            {#if frozen}<p class="reason" id={reasonId}>{copy.conn.staleActions}</p>{/if}
          </section>
        {/if}

        {#if consoleMode() !== 'public'}
        <section class="network" aria-labelledby="{headingId}-wg">
          <h2 class="t-heading" id="{headingId}-wg">{copy.wgImport.heading}</h2>
          <p class="reason">{copy.wgImport.intro}</p>
          <ImportedExits />
          <div>
            <Button onclick={() => openOverlay('sheet', 'wireguard')}>{copy.wgImport.open}</Button>
          </div>
        </section>
        {/if}

        <OfferThisDevice />
      {/if}
    </div>
  {/if}
</section>

<ImportWireguard bind:open={() => importing, (open) => !open && closeOverlay('sheet')} />

{#if change && network}
  <AlertDialog
    bind:open={changeOpen}
    title={change.allowed
      ? copy.exits.allowTitle(change.exit.name, network.name)
      : copy.exits.stopTitle(change.exit.name)}
    description={change.allowed
      ? copy.exits.allowBody(change.exit.name, network.name)
      : change.exit.in_use_by.length > 0
        ? copy.exits.stopUsers(people(change.exit.in_use_by), change.exit.name)
        : copy.exits.stopNobody(change.exit.name)}
    cancelLabel={copy.action.cancel}
    confirmLabel={change.allowed
      ? copy.exits.allowSubmit(change.exit.name)
      : copy.exits.stopSubmit(change.exit.name)}
    confirmVariant="primary"
    onConfirm={apply}
  >
    <p class="consequence">
      {change.allowed ? copy.exits.allowSees(change.exit.name) : copy.exits.stopAgain}
    </p>
  </AlertDialog>
{/if}

<style>
  .screen {
    display: grid;
    gap: var(--s-5);
    max-width: var(--content-max);
    margin: 0;
    padding: var(--s-6) var(--content-pad);
  }
  .sections {
    display: grid;
    gap: var(--s-7);
  }
  .network {
    display: grid;
    gap: var(--s-3);
  }
  .table {
    /* §5 DeviceRow's 84 px rate column, from the spacing scale. */
    --rate-col: calc(var(--s-8) + var(--s-4) + var(--s-1));
    display: grid;
  }
  .row {
    display: grid;
    grid-template-columns:
      minmax(0, 1.2fr) minmax(0, 1fr) minmax(0, 1fr) var(--rate-col)
      var(--control);
    align-items: center;
    gap: var(--s-4);
    min-height: var(--row);
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .row:not(.head):hover {
    background: var(--sunken);
  }
  .head {
    min-height: var(--control);
    color: var(--ink-3);
  }
  .who {
    display: grid;
    min-width: 0;
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .locked .name {
    color: var(--ink-3);
  }
  .status {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .failed {
    color: var(--danger);
  }
  .access,
  .use {
    min-width: 0;
    color: var(--ink-2);
  }
  .hint {
    color: var(--ink-3);
  }
  .quiet {
    color: var(--ink-3);
  }
  .cell-label {
    display: none;
  }
  .num {
    text-align: right;
  }
  .more {
    display: flex;
    justify-content: flex-end;
  }
  .reason {
    color: var(--ink-3);
  }
  .consequence {
    color: var(--ink-2);
  }
  @media (width < 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
    .head {
      display: none;
    }
    .row {
      grid-template-columns: minmax(0, 1fr) var(--rate-col) var(--control);
      grid-template-areas:
        'who num more'
        'access access more'
        'use use more';
      gap: var(--s-1) var(--s-3);
      padding: var(--s-2) 0;
    }
    .who {
      grid-area: who;
    }
    .num {
      grid-area: num;
    }
    .more {
      grid-area: more;
      align-self: start;
    }
    .access {
      grid-area: access;
    }
    .use {
      grid-area: use;
    }
    .access,
    .use {
      display: flex;
      gap: var(--s-2);
      font: var(--t-small);
    }
    .cell-label {
      display: inline;
      color: var(--ink-3);
    }
  }
</style>
