<!--
@component
ImportedExits: every WireGuard network imported from a wg-quick config (`status.imported`),
with what it carries, whether its traffic goes through it now, and when its server was last
heard from. `Use` and `Stop using` go through `POST /api/wireguard/:net/use`; using one that
carries all internet traffic replaces the exit in use, which the button's description says
first. `Remove…` (tier 3) asks in an AlertDialog, then waits for the OS prompt. Only an OS
user who may change kith here (`status.operator`) gets the controls.
Used by Exits.
Props:
  none
-->
<script lang="ts">
  import { announce } from '../announce';
  import { errorText } from '../api';
  import { copy } from '../copy';
  import { ago } from '../format';
  import { actions, live } from '../live/store.svelte';
  import type { ActionHandle } from '../types';
  import type { ImportedInfo, Outcome } from '../types.gen';
  import AlertDialog from '../ui/AlertDialog.svelte';
  import Button from '../ui/Button.svelte';
  import DropdownMenu from '../ui/DropdownMenu.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import StatusDot from '../ui/StatusDot.svelte';
  import { toast } from '../ui/Toast.svelte';

  /** A server silent this long reads as offline, as a plain WireGuard peer does. */
  const SILENT_SECS = 180;

  const imports = $derived(
    [...(live.status?.imported ?? [])].sort((a, b) => a.name.localeCompare(b.name)),
  );
  const operator = $derived(live.status?.operator ?? false);
  const frozen = $derived(live.conn === 'stale' || live.conn === 'daemon_down');
  const exitInUse = $derived(live.status?.exit_node ?? null);

  let busy = $state<Record<string, ActionHandle<Outcome>>>({});
  let removing = $state<ImportedInfo | null>(null);
  let removeOpen = $state(false);

  function heard(one: ImportedInfo): boolean {
    return one.active && one.last_seen_secs !== null && one.last_seen_secs < SILENT_SECS;
  }

  function track(one: ImportedInfo, handle: ActionHandle<Outcome>, done: string): void {
    busy = { ...busy, [one.network]: handle };
    void handle.done.then((h) => {
      if (h.state === 'accepted') {
        toast.done(done);
        busy = Object.fromEntries(Object.entries(busy).filter(([id]) => id !== one.network));
      } else if (h.error) {
        announce(errorText(h.error));
      }
    });
  }

  function use(one: ImportedInfo, active: boolean): void {
    if (frozen) return;
    track(
      one,
      actions.useImport(one.network, active),
      active ? copy.wgImport.used(one.name) : copy.wgImport.stopped(one.name),
    );
  }

  function remove(): void {
    const one = removing;
    removeOpen = false;
    if (one === null || frozen) return;
    track(one, actions.removeImport(one.network), copy.wgImport.removed(one.name));
  }

  function progress(handle: ActionHandle<Outcome>): string | null {
    switch (handle.state) {
      case 'pending':
        return copy.action.signing;
      case 'awaiting_os_auth':
        return copy.action.awaitingOsAuth;
      case 'failed':
      case 'conflict':
        return handle.error ? errorText(handle.error) : null;
      default:
        return null;
    }
  }

  const headingId = $props.id();
</script>

{#if imports.length > 0}
  <ul class="list" aria-label={copy.wgImport.heading}>
    {#each imports as one (one.network)}
      {@const handle = busy[one.network]}
      {@const note = handle ? progress(handle) : null}
      <li class="row">
        <span class="who">
          <span class="line">
            <StatusDot state={heard(one) ? (frozen ? 'stale' : 'online') : 'offline'} />
            <span class="name">{one.name}</span>
            <span class="badge" class:quiet={!one.active}>
              {one.active ? copy.wgImport.inUse : copy.wgImport.notInUse}
            </span>
          </span>
          <span class="detail">
            {one.everything ? copy.wgImport.everything : copy.wgImport.routes(one.routes)}{#if one.endpoint}<span
                class="mono"> · {one.endpoint}</span
              >{/if}
          </span>
          <span class="detail">
            {one.last_seen_secs === null
              ? copy.wgImport.neverHeard
              : copy.wgImport.heard(ago(one.last_seen_secs))}
          </span>
          {#if note}
            <span class="status" class:failed={handle?.state === 'failed'} role="status">{note}</span>
          {/if}
        </span>
        {#if operator}
          <span class="actions">
            {#if one.active}
              <Button
                variant="secondary"
                disabled={frozen}
                pending={handle?.state === 'pending'}
                onclick={() => use(one, false)}
              >
                {copy.wgImport.stop(one.name)}
              </Button>
            {:else}
              <Button
                variant="secondary"
                disabled={frozen}
                pending={handle?.state === 'pending'}
                aria-describedby={one.everything && exitInUse ? `${headingId}-replaces` : undefined}
                onclick={() => use(one, true)}
              >
                {copy.wgImport.use(one.name)}
              </Button>
            {/if}
            <DropdownMenu
              items={[
                {
                  label: copy.wgImport.remove,
                  tone: 'danger',
                  onSelect: () => {
                    removing = one;
                    removeOpen = true;
                  },
                },
              ]}
            >
              {#snippet trigger(props)}
                <IconButton icon="ellipsis" label={copy.wgImport.moreFor(one.name)} triggerProps={props} />
              {/snippet}
            </DropdownMenu>
          </span>
        {/if}
      </li>
    {/each}
  </ul>
  {#if exitInUse && imports.some((one) => one.everything && !one.active)}
    <p class="quiet" id="{headingId}-replaces">{copy.wgImport.replacesExit}</p>
  {/if}
{/if}

{#if removing}
  <AlertDialog
    bind:open={removeOpen}
    title={copy.wgImport.removeTitle(removing.name)}
    description={copy.wgImport.removeBody(removing.name)}
    cancelLabel={copy.action.cancel}
    confirmLabel={copy.wgImport.removeSubmit(removing.name)}
    confirmVariant="danger"
    onConfirm={remove}
  />
{/if}

<style>
  .list {
    display: grid;
    gap: var(--s-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    min-height: var(--row);
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-2);
  }
  .row:hover {
    background: var(--sunken);
  }
  .who {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .name {
    font-weight: 600;
  }
  .badge {
    font: var(--t-small);
    color: var(--ink-2);
  }
  .detail,
  .quiet,
  .status {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .mono {
    font-family: var(--ff-mono);
  }
  .failed {
    color: var(--danger);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
</style>
