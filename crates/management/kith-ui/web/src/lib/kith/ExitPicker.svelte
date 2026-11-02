<!--
@component
ExitPicker (spec §6.7, §7.8): where this device sends its internet traffic. Every option is
visible as a radio: `No exit`, then each exit it may use with its latency. An exit it has no
access to gets a plain row instead: admins see `Allow` (tier 2, an AlertDialog), members see
`Ask an admin`.

Switching is optimistic: the radio moves at once and the row reads `Switching…`. When the
server accepts, the line under the list reads `Your internet now goes through vps.`; while
the OS asks for confirmation the row says so, and the line waits for the daemon to report the
change. When it fails, the radio moves back and the line says why. One switch at a time: the
other options are disabled until it settles. Arrow keys move focus only; Space or Enter
picks, so walking the list never switches exits by accident (the group says so to screen
readers).

Used by the Exits screen, the header's ExitChip popover and ⌘K `Use exit…`.
Props:
  - net: string, the network's id or name (as in the URL).
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { announce } from '../announce';
  import { errorText } from '../api';
  import { copy } from '../copy';
  import { rtt } from '../format';
  import { actions, live } from '../live/store.svelte';
  import { BANNER_DELAY_MS } from '../live/timing';
  import type { ActionHandle, ApiFailure } from '../types';
  import type { ExitView, NetworkSummary, Outcome } from '../types.gen';
  import AlertDialog from '../ui/AlertDialog.svelte';
  import Button from '../ui/Button.svelte';
  import CopyField from '../ui/CopyField.svelte';
  import AskAdminSheet from './AskAdminSheet.svelte';

  let { net }: { net: string } = $props();

  /** The `No exit` option's key; every other option is keyed by its device id. */
  const NONE = '';

  type Pick = { key: string; name: string | null };
  type Note = { tone: 'done' | 'failed'; text: string; key: string | null };

  function findNetwork(list: readonly NetworkSummary[], ref: string): NetworkSummary | null {
    return (
      list.find((n) => n.id === ref) ??
      list.find((n) => n.name === ref) ??
      (ref === '' ? undefined : list.find((n) => n.id.startsWith(ref))) ??
      null
    );
  }

  const byName = (a: ExitView, b: ExitView): number => a.name.localeCompare(b.name);

  const network = $derived(findNetwork(live.status?.networks ?? [], net));
  const netId = $derived(network?.id ?? null);
  const admin = $derived(network?.admin ?? false);
  const self = $derived(live.status?.node ?? null);
  const peers = $derived(netId ? Object.values(live.peers[netId] ?? {}) : []);
  const exits = $derived(netId ? (live.exits[netId] ?? null) : null);
  const others = $derived((exits ?? []).filter((e) => e.node !== self));
  const usable = $derived(others.filter((e) => e.allowed).sort(byName));
  const locked = $derived(others.filter((e) => !e.allowed).sort(byName));

  // The exit in use, from `exit_node` (`network/device`, by name).
  const inUse = $derived.by(() => {
    const value = live.status?.exit_node ?? null;
    if (value === null) return null;
    const slash = value.indexOf('/');
    return { network: value.slice(0, slash), name: value.slice(slash + 1) };
  });
  const inUseHere = $derived(
    inUse !== null &&
      network !== null &&
      (inUse.network === network.name || inUse.network === network.id),
  );
  /** The option the daemon reports; `null` when the exit in use is in another network. */
  const truthKey = $derived.by(() => {
    if (inUse === null) return NONE;
    if (!inUseHere) return null;
    const node =
      usable.find((e) => e.name === inUse.name)?.node ??
      peers.find((p) => p.name === inUse.name)?.node;
    return node ?? null;
  });

  // §8.6: exit choice is optimistic. `pick` holds the choice until the daemon reports it.
  let pick = $state<Pick | null>(null);
  let switching = $state(false);
  /** The server took the switch and the OS is asking to confirm it. */
  let awaiting = $state(false);
  let note = $state<Note | null>(null);
  let attempt = 0;

  const selectedKey = $derived(pick?.key ?? truthKey);

  $effect(() => {
    // Confirmed on this computer: the daemon now reports the pick.
    if (!awaiting || pick === null || truthKey !== pick.key) return;
    const text = pick.name === null ? copy.exits.doneNone : copy.exits.done(pick.name);
    awaiting = false;
    switching = false;
    note = { tone: 'done', text, key: pick.key };
    announce(text);
  });
  $effect(() => {
    if (pick !== null && !switching && truthKey === pick.key) pick = null;
  });
  $effect(() => {
    // Someone else changed the exit: the last result no longer describes it.
    if (note?.tone === 'done' && pick === null && truthKey !== note.key) note = null;
  });

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

  let loadFailure = $state<ApiFailure | null>(null);
  function reload(id: string): void {
    const handle = actions.loadExits(id);
    void handle.done.then((h) => (loadFailure = h.state === 'failed' ? h.error : null));
  }
  $effect(() => {
    const id = netId;
    if (id !== null) untrack(() => reload(id));
  });

  /** Options when the list could not load: no exit, and the one in use. */
  const fallback = $derived.by(() => {
    if (exits !== null || loadFailure === null || truthKey === null || truthKey === NONE) return [];
    return inUse ? [{ node: truthKey, name: inUse.name }] : [];
  });
  const options = $derived([
    { key: NONE, node: null as string | null, name: null as string | null },
    ...(exits !== null ? usable : fallback).map((e) => ({ key: e.node, node: e.node, name: e.name })),
  ]);
  const ready = $derived(exits !== null || loadFailure !== null);

  const currentName = $derived(inUse?.name ?? null);

  /**
   * §7.8 step 4. An exit that can't be reached (the daemon answers not_found for it) gets
   * the reachability sentence; any other cause is the §12.5 text. Either way the line ends
   * with what is still in use, read when the answer arrives.
   */
  function failure(handle: ActionHandle<Outcome>, target: string | null): string {
    const still = currentName;
    const error = handle.error;
    if (target !== null && error?.code === 'not_found') {
      return still === null ? copy.exits.failedNone(target) : copy.exits.failed(target, still);
    }
    const cause =
      handle.state === 'conflict' && handle.conflict
        ? copy.error.conflict(handle.conflict.resolved_by)
        : error
          ? errorText(error, { subject: target ?? undefined })
          : copy.error.internal('');
    return copy.exits.failedOther(target, cause, still);
  }

  function choose(key: string, node: string | null, name: string | null): void {
    if (frozen || netId === null || switching) return;
    if (key === selectedKey) return;
    attempt += 1;
    const mine = attempt;
    pick = { key, name };
    switching = true;
    awaiting = false;
    note = null;
    const handle = actions.useExit(netId, node);
    void handle.done.then((h) => {
      if (mine !== attempt) return;
      if (h.state === 'awaiting_os_auth') {
        // Nothing has changed yet: the row says what it waits for (§8.6).
        awaiting = true;
        return;
      }
      switching = false;
      if (h.state === 'accepted' || h.state === 'seen') {
        const text = name === null ? copy.exits.doneNone : copy.exits.done(name);
        note = { tone: 'done', text, key };
        announce(text);
      } else {
        pick = null;
        const text = failure(h, name);
        note = { tone: 'failed', text, key: null };
        announce(text);
      }
    });
  }

  function label(option: { key: string; node: string | null; name: string | null }): string {
    const moving = switching && pick?.key === option.key;
    const doing = awaiting ? copy.action.awaitingOsAuth : copy.exits.switching;
    if (option.node === null || option.name === null) {
      return moving ? copy.exits.option(copy.exits.noneShort, doing) : copy.exits.none;
    }
    if (moving) return copy.exits.option(option.name, doing);
    if (option.key === truthKey && pick === null) {
      if (live.status?.exit_state === 'offline_blocked') return copy.header.exitOfflineBlocked(option.name);
      if (live.status?.exit_state === 'offline_direct') return copy.header.exitOfflineDirect(option.name);
    }
    const peer = peers.find((p) => p.node === option.node);
    if (peer && !peer.online) return copy.exits.option(option.name, copy.status.offline);
    return copy.exits.option(option.name, rtt(live.numbers[`${netId}/${option.node}`]?.rttMs ?? null));
  }

  function tone(option: { key: string; node: string | null }): 'danger' | 'quiet' | null {
    if (option.node === null) return null;
    if (option.key === truthKey && pick === null && live.status?.exit_state?.startsWith('offline')) {
      return 'danger';
    }
    const peer = peers.find((p) => p.node === option.node);
    return peer && !peer.online ? 'quiet' : null;
  }

  // Roving focus (§10: every pointer action has a keyboard path).
  let group: HTMLDivElement | null = $state(null);
  let focused = $state<string | null>(null);
  const tabKey = $derived(
    options.some((o) => o.key === focused)
      ? focused
      : options.some((o) => o.key === selectedKey)
        ? selectedKey
        : (options[0]?.key ?? NONE),
  );

  function focusAt(index: number): void {
    const count = options.length;
    const target = options[((index % count) + count) % count];
    if (!target || !group) return;
    group.querySelector<HTMLElement>(`[data-key="${CSS.escape(target.key)}"]`)?.focus();
  }

  function keydown(event: KeyboardEvent, index: number): void {
    const option = options[index];
    switch (event.key) {
      case 'ArrowDown':
      case 'ArrowRight':
        focusAt(index + 1);
        break;
      case 'ArrowUp':
      case 'ArrowLeft':
        focusAt(index - 1);
        break;
      case 'Home':
        focusAt(0);
        break;
      case 'End':
        focusAt(options.length - 1);
        break;
      case ' ':
      case 'Enter':
        if (option) choose(option.key, option.node, option.name);
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  // Tier 2: allowing an exit shows its consequences first (§13).
  let allowing = $state<ExitView | null>(null);
  let allowOpen = $state(false);
  let grants = $state<Record<string, ActionHandle<Outcome>>>({});

  function ask(exit: ExitView): void {
    allowing = exit;
    allowOpen = true;
  }

  function allow(): void {
    const exit = allowing;
    allowOpen = false;
    if (exit === null || netId === null) return;
    const id = netId;
    const handle = actions.grantExit(id, exit.node, true);
    grants = { ...grants, [exit.node]: handle };
    void handle.done.then((h) => {
      if (h.state === 'accepted') reload(id);
      else announce(grantFailure(h, exit.name));
    });
  }

  function grantFailure(handle: ActionHandle<Outcome>, exit: string): string {
    if (handle.state === 'conflict' && handle.conflict) {
      return copy.error.conflict(handle.conflict.resolved_by);
    }
    switch (handle.error?.code) {
      case 'permission_denied':
        return copy.error.permissionDenied;
      case 'daemon_down':
        return copy.conn.staleActions;
      case 'not_implemented':
        return copy.error.notImplemented;
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
        return grantFailure(handle, exit);
      default:
        if (handle.seen) return copy.action.spreading(handle.seen.by, handle.seen.of);
        return handle.slow ? copy.action.savedWaiting : copy.action.savedHere;
    }
  }

  // Members: the same place offers the request to send an admin (§7.10).
  let askOpen = $state(false);
  let askRequest = $state('');
  function askAdmin(exit: ExitView): void {
    askRequest = copy.askAdmin.request.exitAllow(network?.name ?? net, exit.name);
    askOpen = true;
  }

  const headingId = $props.id();
  const reasonId = `${headingId}-reason`;
  const noteId = `${headingId}-note`;
  const hintId = `${headingId}-hint`;
  const describedBy = $derived(
    [hintId, frozen ? reasonId : '', !frozen && note?.tone === 'failed' ? noteId : '']
      .filter(Boolean)
      .join(' '),
  );

  /** `You don't have access to ` and `.`, around a name that must not break. */
  const [noAccessBefore = '', noAccessAfter = ''] = copy.exits.noAccess('\u0000').split('\u0000');
</script>

<div class="picker">
  <h2 class="t-heading" id={headingId}>{copy.exits.heading}</h2>

  {#if ready}
    <span class="sr-only" id={hintId}>{copy.exits.switchHint}</span>
    <div
      bind:this={group}
      class="options"
      role="radiogroup"
      aria-labelledby={headingId}
      aria-describedby={describedBy}
    >
      {#each options as option, index (option.key)}
        {@const checked = option.key === selectedKey}
        <div
          class="option {tone(option) ?? ''}"
          class:tint={checked && live.changed.has('exit')}
          role="radio"
          aria-checked={checked}
          aria-disabled={frozen || (switching && !checked) ? 'true' : undefined}
          tabindex={option.key === tabKey ? 0 : -1}
          data-key={option.key}
          onclick={() => choose(option.key, option.node, option.name)}
          onkeydown={(event) => keydown(event, index)}
          onfocus={() => (focused = option.key)}
        >
          <span class="mark" aria-hidden="true"></span>
          <span class="label">{label(option)}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if frozen}
    <p class="reason" id={reasonId}>{copy.conn.staleActions}</p>
  {:else if note}
    <p class="note" class:failed={note.tone === 'failed'} id={noteId}>{note.text}</p>
  {/if}

  {#if ready}
    {#if locked.length > 0}
      <ul class="locked">
        {#each locked as exit (exit.node)}
          {@const grant = grants[exit.node]}
          <li class="lock">
            <span class="mark" aria-hidden="true"></span>
            <div class="lock-text">
              <span>{noAccessBefore}<span class="nowrap">{exit.name}</span>{noAccessAfter}</span>
              {#if grant}
                <span
                  class="status"
                  class:failed={grant.state === 'failed' || grant.state === 'conflict'}
                >
                  {progress(grant, exit.name)}
                </span>
              {/if}
            </div>
            {#if admin}
              <Button
                variant="secondary"
                pending={grant?.state === 'pending'}
                disabled={frozen}
                aria-describedby={frozen ? reasonId : undefined}
                onclick={() => ask(exit)}
              >
                {copy.exits.allow}
              </Button>
            {:else}
              <Button variant="secondary" onclick={() => askAdmin(exit)}>{copy.exits.askAdmin}</Button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    {#if exits !== null && others.length === 0}
      <div class="empty">
        <p>{copy.exits.empty}</p>
        <CopyField value={copy.exits.offerCommand} />
        <p>{copy.exits.emptyThen}</p>
      </div>
    {/if}

    {#if loadFailure && loadFailure.code !== 'not_implemented'}
      <p class="note failed">{copy.error.internal(loadFailure.message)}</p>
    {/if}
  {/if}
</div>

{#if allowing && network}
  <AlertDialog
    bind:open={allowOpen}
    title={copy.exits.allowTitle(allowing.name, network.name)}
    description={copy.exits.allowBody(allowing.name, network.name)}
    cancelLabel={copy.action.cancel}
    confirmLabel={copy.exits.allowSubmit(allowing.name)}
    confirmVariant="primary"
    onConfirm={allow}
  >
    <p class="consequence">{copy.exits.allowSees(allowing.name)}</p>
  </AlertDialog>
{/if}

<AskAdminSheet bind:open={askOpen} {net} request={askRequest} />

<style>
  .picker {
    display: grid;
    gap: var(--s-3);
  }
  .options,
  .locked {
    display: grid;
  }
  .option {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: var(--row);
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .option:hover {
    background: var(--sunken);
  }
  .option:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: calc(-1 * var(--focus-ring));
  }
  .option[aria-disabled='true'] {
    cursor: default;
  }
  .option.quiet .label {
    color: var(--ink-3);
  }
  .option.danger .label {
    color: var(--danger);
  }
  .tint {
    animation: tint var(--d-tint) var(--e-linear);
  }
  @keyframes tint {
    from {
      background-color: var(--changed);
    }
  }
  .mark {
    position: relative;
    flex: none;
    width: var(--icon);
    height: var(--icon);
    border-radius: var(--r-full);
    border: var(--dot-ring) solid var(--line-strong);
    background: var(--surface);
  }
  [aria-checked='true'] > .mark {
    border-color: var(--ink-1);
  }
  [aria-checked='true'] > .mark::after {
    content: '';
    position: absolute;
    inset: 0;
    margin: auto;
    width: var(--dot);
    height: var(--dot);
    border-radius: var(--r-full);
    background: var(--ink-1);
  }
  .label {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .lock {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-start;
    gap: var(--s-3);
    min-height: var(--row);
    padding: 0 var(--s-3);
    color: var(--ink-3);
  }
  .lock > .mark {
    border-color: var(--ink-4);
    background: transparent;
  }
  .lock-text {
    display: grid;
    min-width: 0;
  }
  .nowrap {
    white-space: nowrap;
  }
  .status {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .empty {
    display: grid;
    gap: var(--s-2);
    max-width: var(--measure);
    color: var(--ink-2);
  }
  .note {
    color: var(--ink-2);
    max-width: var(--measure);
  }
  .failed {
    color: var(--danger);
  }
  .reason {
    color: var(--ink-3);
  }
  .consequence {
    color: var(--ink-2);
  }
  @media (forced-colors: active) {
    .mark,
    [aria-checked='true'] > .mark {
      border-color: CanvasText;
    }
    [aria-checked='true'] > .mark::after {
      background: CanvasText;
    }
  }
</style>
