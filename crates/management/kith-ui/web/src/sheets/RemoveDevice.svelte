<!--
@component
RemoveDevice (spec §7.9). Removing another device is tier 2: an AlertDialog with the
consequences, focus on Cancel, Enter not bound, the button naming the device. Removing this
device is leaving, tier 3: the network's name typed, then OS confirmation in the server
(§13). The last admin can't be removed; the dialog says why instead.

While it signs, the dialog stays; once accepted it closes and the device's row carries the
progress (`removalLine`). If someone else removes the device while this is open, it closes
and says nothing was signed.

Props:
  - open?: boolean, bindable. The screen maps it to `?remove=` (Back closes it).
  - net: string, the network's id or name.
  - node: string, the device's id (this device's own id means leaving).
  - name: string, the device's name. network: string, the network's name.
  - onOpenChange?: (open: boolean) => void.

Module exports, for DeviceRow (spec §5 Removing state):
  - removalOf(net, node): the removal in flight for a device, or null. `net` is the id.
  - removalLine(removal): `Removing… 3 of 5 have dropped phone`, then
    `2 offline devices will drop it when they're back.`
-->
<script lang="ts" module>
  import { SvelteMap } from 'svelte/reactivity';
  import { copy as text } from '../lib/copy';
  import { live as store } from '../lib/live/store.svelte';
  import { REMOVED_HOLD_MS, ROW_ANIMATION_MS } from '../lib/live/timing';
  import type { ActionHandle } from '../lib/types';
  import type { Outcome } from '../lib/types.gen';

  export type Removal = {
    readonly net: string;
    readonly node: string;
    readonly name: string;
    readonly handle: ActionHandle<Outcome>;
    /** Devices that were offline when it was signed: they drop it when they're back. */
    readonly offline: number;
  };

  const removals = new SvelteMap<string, Removal>();
  let watching = false;

  /** Forgets each removal once its device has left the list and its row has collapsed. */
  function watch(): void {
    if (watching) return;
    watching = true;
    $effect.root(() => {
      $effect(() => {
        for (const [key, removal] of removals) {
          if (store.peers[removal.net]?.[removal.node]) continue;
          setTimeout(() => {
            if (removals.get(key) === removal) removals.delete(key);
          }, REMOVED_HOLD_MS + ROW_ANIMATION_MS);
        }
      });
    });
  }

  function track(removal: Removal): void {
    const key = `${removal.net}/${removal.node}`;
    removals.set(key, removal);
    watch();
    void removal.handle.done.then((handle) => {
      if (handle.state !== 'accepted' && removals.get(key) === removal) removals.delete(key);
    });
  }

  export function removalOf(net: string, node: string): Removal | null {
    return removals.get(`${net}/${node}`) ?? null;
  }

  export function removalLine(removal: Removal): string {
    const seen = removal.handle.seen;
    if (!seen) return text.device.removing;
    if (seen.by < seen.of) return text.remove.progress(seen.by, seen.of, removal.name);
    return removal.offline > 0 ? text.remove.offlineLater(removal.offline) : text.device.removed;
  }
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import { announce } from '../lib/announce';
  import { copy } from '../lib/copy';
  import { actions, live } from '../lib/live/store.svelte';
  import { go } from '../router';
  import AlertDialog from '../lib/ui/AlertDialog.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import { cannotAct, failureText, networkContext } from './act';

  let {
    open = $bindable(false),
    net,
    node,
    name,
    network,
    onOpenChange,
  }: {
    open?: boolean;
    net: string;
    node: string;
    name: string;
    network: string;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const context = $derived(networkContext(live.status, net));
  const netId = $derived(context?.id ?? net);
  const self = $derived(node === live.status?.node);
  const peer = $derived(live.peers[netId]?.[node] ?? null);
  const address = $derived(self ? (context?.membership?.address ?? '') : (peer?.address ?? ''));
  const admins = $derived(context?.membership?.admins ?? []);
  const lastAdmin = $derived(admins.length === 1 && admins[0] === name);
  const stale = $derived(cannotAct(live.conn));

  /** `laptop and tv use phone as their exit.` Every device but the one being removed. */
  const exitUsers = $derived.by(() => {
    const me = context?.membership?.name ?? null;
    const users =
      (live.exits[netId] ?? []).find((e) => e.node === node)?.in_use_by.filter((n) => n !== name) ??
      [];
    // This device reads `this device`, first, so the person sees their own machine.
    if (me === null || !users.includes(me)) return users;
    return [copy.device.thisDevice, ...users.filter((n) => n !== me)];
  });
  const exitLine = $derived.by(() => {
    const line = copy.remove.exitUsers(exitUsers, name);
    return exitUsers[0] === copy.device.thisDevice
      ? line.charAt(0).toUpperCase() + line.slice(1)
      : line;
  });

  let lost = $state(false);
  let typed = $state('');
  let handle = $state.raw<ActionHandle<Outcome> | null>(null);
  const pending = $derived(handle?.state === 'pending' || handle?.state === 'awaiting_os_auth');
  /** A device already gone is not a failure: the dialog closes and says so instead. */
  const failure = $derived(
    handle?.state === 'failed' && handle.error && handle.error.code !== 'not_found'
      ? failureText(handle.error, name)
      : null,
  );

  $effect(() => {
    if (failure) announce(failure);
  });

  /** The race below fires only once the device was there while this was open. */
  let seenPeer = false;
  $effect(() => {
    if (!open) return;
    untrack(() => {
      seenPeer = false;
      lost = false;
      typed = '';
      handle = null;
      if (!live.exits[netId] && !self) actions.loadExits(netId);
    });
  });

  /** §7.9 step 4: removed by someone else while this was open. */
  $effect(() => {
    if (!open || self || handle !== null) return;
    if (peer !== null) {
      seenPeer = true;
      return;
    }
    if (!seenPeer) return;
    const who = untrack(() => name);
    open = false;
    // The journal's `device_removed` carries no signer, so the toast cannot name who.
    toast.done(copy.remove.racedPlain(who));
  });

  let last: boolean | null = null;
  $effect(() => {
    const now = open;
    if (last !== null && now !== last) untrack(() => onOpenChange?.(now));
    last = now;
  });

  function settle(next: ActionHandle<Outcome>): void {
    void next.done.then((result) => {
      if (handle !== next) return;
      const who = name;
      if (result.state === 'accepted') {
        open = false;
        if (self) go('/');
        else {
          const removal = removalOf(netId, node);
          if (removal) announce(removalLine(removal));
        }
      } else if (result.state === 'conflict' && result.conflict) {
        open = false;
        toast.done(copy.remove.raced(who, result.conflict.resolved_by));
      } else if (result.state === 'conflict' || result.error?.code === 'not_found') {
        open = false;
        toast.done(copy.remove.racedPlain(who));
      }
    });
  }

  function remove(): void {
    if (stale || pending) return;
    const next = actions.removeDevice(netId, node, lost);
    handle = next;
    track({
      net: netId,
      node,
      name,
      handle: next,
      offline: Object.values(live.peers[netId] ?? {}).filter((p) => p.node !== node && !p.online)
        .length,
    });
    settle(next);
  }

  const confirmed = $derived(typed.trim() === network);

  function leave(): void {
    if (stale || pending || !confirmed) return;
    const next = actions.leave(netId);
    handle = next;
    settle(next);
  }
</script>

{#if lastAdmin}
  <Dialog
    bind:open
    title={self ? copy.remove.leaveTitle(network) : copy.remove.title(name, network)}
  >
    <p class="body">{copy.remove.lastAdmin(name)}</p>
    {#snippet footer()}
      <Button onclick={() => (open = false)}>{copy.action.close}</Button>
    {/snippet}
  </Dialog>
{:else if self}
  <AlertDialog
    bind:open
    title={copy.remove.leaveTitle(network)}
    description={copy.remove.leaveLoses(network)}
    cancelLabel={copy.action.cancel}
    confirmLabel={copy.remove.leaveSubmit(network)}
    confirmDisabled={stale || !confirmed}
    {pending}
    onConfirm={leave}
  >
    <p class="body">{copy.remove.comeBack}</p>
    <Field label={copy.settings.typeToConfirm(network)}>
      {#snippet children(control)}
        <Input {...control} kind="name" bind:value={typed} autocomplete="off" />
      {/snippet}
    </Field>
    {#if pending}<p class="note">{copy.tier3.waiting}</p>{/if}
    {#if failure}<p class="fail">{failure}</p>{/if}
    {#if stale}<p class="note">{copy.conn.staleActions}</p>{/if}
  </AlertDialog>
{:else}
  <AlertDialog
    bind:open
    title={copy.remove.title(name, network)}
    description={copy.remove.loses(name)}
    cancelLabel={copy.action.cancel}
    confirmLabel={copy.remove.submit(name)}
    confirmDisabled={stale}
    {pending}
    onConfirm={remove}
  >
    <div class="body">
      {#if address}
        <p>{copy.remove.frees(address, `${name}${copy.names.suffix(network)}`)}</p>
      {/if}
      <p>{copy.remove.comeBack}</p>
    </div>
    <label class="check">
      <input type="checkbox" bind:checked={lost} />
      <span>{copy.remove.lost(name)}</span>
    </label>
    {#if exitUsers.length > 0}
      <p class="body">{exitLine}</p>
    {/if}
    {#if failure}<p class="fail">{failure}</p>{/if}
    {#if stale}<p class="note">{copy.conn.staleActions}</p>{/if}
  </AlertDialog>
{/if}

<style>
  .body {
    display: grid;
    gap: var(--s-1);
    color: var(--ink-2);
  }
  .check {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    cursor: pointer;
  }
  .check input {
    flex: none;
    width: var(--icon);
    height: var(--icon);
    margin-top: calc((1lh - var(--icon)) / 2);
    accent-color: var(--accent);
  }
  .check input:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .note {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .fail {
    color: var(--danger);
  }
</style>
