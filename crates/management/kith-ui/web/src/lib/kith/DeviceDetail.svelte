<!--
@component
DeviceDetail (spec §6.4): everything about one device, sections 32 px apart:
  1. header: name (admins rename it inline, §7.11), status, mono address and full name with Copy;
  2. verdict: one sentence and its cause (§12.4), with an action only when one exists;
  3. path: `This Mac ─ vps ─ nas` with the latency on the line, endpoint, MTU, reason and
     who reported it;
  4. traffic: the chart with the `1m · 15m · 1h` range and the window totals, then, on
     request, bytes per day for the 30 days the Console keeps;
  5. names pointing here, with Copy;
  6. history: the last 10 entries, then `All →` into Activity;
  6b. internet traffic (DeviceSettings): the exit it uses, Lock and Pause;
  7. actions: admins `Rename`, `Use as exit` or `Allow as exit…`, then `Remove from home…`;
     members one `Ask an admin…`; this device `Leave home…` instead of Remove.
While the connection is stale every change is aria-disabled with the reason inline.
The container (split panel, drawer or full page) belongs to the Devices screen.
Props:
  - net: string, network id.
  - node: string, the device id.
  - name?: string, the name the URL used, for `tv is no longer in this network.`
  - mode?: 'split' | 'drawer' | 'page' (default 'split'): `page` shows a back arrow,
    the others a close button.
  - range?: Range (default '1h'), from `?range=`.
  - at?: number | null, server ms from `?at=`: the chart's cursor parks there (§6.5).
  - stale?: boolean, values are last known (§8.5).
  - autofocus?: boolean, focus the name when mounted (drawer and page).
  - onClose?: () => void.
-->
<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { MediaQuery } from 'svelte/reactivity';
  import { connection } from '../../frame/conn.svelte';
  import { go, href, setQuery } from '../../router';
  import { fullySeen } from '../../sheets/act';
  import AllowBlocked from '../../sheets/AllowBlocked.svelte';
  import { removalLine, removalOf } from '../../sheets/removals.svelte';
  import { announce } from '../announce';
  import Series from '../charts/Series.svelte';
  import TrafficDays from './TrafficDays.svelte';
  import { copy } from '../copy';
  import { bytes, rtt as formatRtt } from '../format';
  import Icon from '../icons/Icon.svelte';
  import { actions, live } from '../live/store.svelte';
  import {
    COPIED_MS,
    RANGE_MS,
    RELATIVE_TIME_REFRESH_MS,
    SPACES_HINT_MS,
  } from '../live/timing';
  import type { ActionHandle, ApiFailure, Range } from '../types';
  import type { LinkTick, NameCheck, Outcome } from '../types.gen';
  import AlertDialog from '../ui/AlertDialog.svelte';
  import Badge from '../ui/Badge.svelte';
  import Button from '../ui/Button.svelte';
  import DropdownMenu, { type MenuItem } from '../ui/DropdownMenu.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Field from '../ui/Field.svelte';
  import Input from '../ui/Input.svelte';
  import Segmented from '../ui/Segmented.svelte';
  import StatusDot from '../ui/StatusDot.svelte';
  import { toast } from '../ui/Toast.svelte';
  import AskAdminSheet from './AskAdminSheet.svelte';
  import { askRenameRequest, askRequest as rowAsk, renameRequest, snapshotSeenAt } from './deviceState.svelte';
  import {
    agoMs,
    blockedTo,
    eventsOf,
    fqdn,
    historyOf,
    isFlapping,
    isPhone,
    isRelayed,
    joinedAt,
    lastSeenAt,
    membershipOf,
    namesPointingAt,
    presenceOf,
    relayName,
    selfPeer,
    sentence,
    verdict as verdictOf,
    windowTotals,
  } from './devices';
  import DeviceSettings from './DeviceSettings.svelte';
  import EventRow from './EventRow.svelte';
  import PathBadge from './PathBadge.svelte';

  let {
    net,
    node,
    name = '',
    mode = 'split',
    range = '1h',
    at = null,
    stale,
    autofocus = false,
    onClose,
  }: {
    net: string;
    node: string;
    name?: string;
    mode?: 'split' | 'drawer' | 'page';
    range?: Range;
    at?: number | null;
    stale?: boolean;
    autofocus?: boolean;
    onClose?: () => void;
  } = $props();

  const reducedMotion = new MediaQuery('prefers-reduced-motion: reduce');
  const HISTORY_LIMIT = 10;
  const titleId = $props.id();

  let now = $state(live.now());
  onMount(() => {
    const timer = setInterval(() => (now = live.now()), RELATIVE_TIME_REFRESH_MS);
    return () => clearInterval(timer);
  });

  const network = $derived(live.status?.networks.find((n) => n.id === net) ?? null);
  const networkName = $derived(network?.name ?? '');
  const admin = $derived(network?.admin ?? false);
  const membership = $derived(membershipOf(live.status, net));
  const selfNode = $derived(live.status?.node ?? null);
  const isSelf = $derived(node === selfNode);
  const peer = $derived(
    isSelf ? selfPeer(live.status, membership) : (live.peers[net]?.[node] ?? null),
  );
  const key = $derived(`${net}/${node}`);
  const isStale = $derived(stale ?? connection.stale);
  const numbers = $derived(live.numbers[key] ?? null);
  const relay = $derived(relayName(live.peers[net], numbers?.relay ?? null));
  /** The device it connects through (a phone's gateway), with this device in words. */
  const gatewayName = $derived(
    peer?.gateway == null
      ? null
      : peer.gateway === membership?.name
        ? copy.device.thisDevice
        : peer.gateway,
  );
  /** Whether the URL's name was ever in this network, for `no longer` versus `no device`. */
  const seenBefore = $derived(
    live.journal.some(
      (e) =>
        (e.kind === 'device_removed' || e.kind === 'device_joined') &&
        e.network === net &&
        e.name === name,
    ),
  );
  /** The daemon's reason with this device in words: `Connects through this device.` */
  function pathReason(text: string): string {
    const me = membership?.name;
    if (!me) return text;
    // Device names are lowercase letters, digits and hyphens: nothing to escape.
    return text.replace(new RegExp(`\\b${me}\\b`, 'g'), copy.device.thisDevice);
  }
  /** This device in words when it is the one reporting or relaying (§12.1, §12.2). */
  function who(name: string): string {
    return name === membership?.name ? copy.device.thisDevice : name;
  }
  const seenAt = $derived(
    peer ? lastSeenAt(live.journal, net, peer, snapshotSeenAt(key, peer.last_seen_secs, now)) : null,
  );
  const presence = $derived(
    peer
      ? isSelf
        ? ({ kind: 'online' } as const)
        : presenceOf(peer, seenAt, joinedAt(live.journal, net, node))
      : null,
  );
  const online = $derived(presence?.kind === 'online');
  /** The tint's stamp: a second change inside 4 s restarts the fade (§8.4). */
  const stamp = $derived(live.changed.get(key) ?? null);
  const exitName = $derived(peer ? `${networkName}/${peer.name}` : '');

  // §12.4 verdict.
  const blocked = $derived(blockedTo(live.pending.blocked, net, node));
  const verdict = $derived.by(() => {
    if (!peer || isSelf) return null;
    const seenAgoMs = !peer.online
      ? seenAt === null
        ? null
        : now - seenAt
      : peer.last_seen_secs === null
        ? null
        : // Traffic in the last tick means it is not quiet, whatever the snapshot said.
          (numbers?.rx ?? 0) > 0
          ? 0
          : peer.last_seen_secs * 1000;
    return verdictOf({
      peer,
      relay,
      gateway: gatewayName,
      rttMs: numbers?.rttMs ?? null,
      seenAgoMs,
      blocked,
    });
  });
  let whyOpen = $state(false);

  // §6.4 Path.
  const ownLink = $derived(
    (live.links[net] ?? []).find((l) => l.from === selfNode && l.to === node) ?? null,
  );
  const reverseLink = $derived(
    (live.links[net] ?? []).find((l) => l.from === node && l.to === selfNode) ?? null,
  );
  const reporter: { link: LinkTick; who: string } | null = $derived.by(() => {
    if (reverseLink && peer) return { link: reverseLink, who: peer.name };
    if (ownLink && membership) return { link: ownLink, who: copy.device.thisDevice };
    return null;
  });
  const selfLabel = $derived(
    live.status?.host.os === 'macOS' ? copy.deviceView.selfMac : copy.deviceView.selfDevice,
  );
  type Hop = { label: string; style: 'direct' | 'relayed' | 'gateway' };
  const hops: Hop[] = $derived.by(() => {
    if (!peer || isSelf || !online) return [];
    const via = peer.gateway;
    if (via !== null) {
      if (via === membership?.name) return [{ label: peer.name, style: 'gateway' }];
      return [
        { label: via, style: 'direct' },
        { label: peer.name, style: 'gateway' },
      ];
    }
    if (isRelayed(peer.path)) {
      return [
        { label: relay ?? copy.deviceView.relayNode, style: 'relayed' },
        { label: peer.name, style: 'relayed' },
      ];
    }
    return [{ label: peer.name, style: 'direct' }];
  });
  const rttMs = $derived(online ? (numbers?.rttMs ?? null) : null);
  /** The daemon sends `relay` as a placeholder; only a real host:port shows, in mono. */
  const endpoint = $derived.by(() => {
    if (peer?.endpoint && /:\d+$/.test(peer.endpoint)) return { via: null, at: peer.endpoint };
    const relayNode = numbers?.relay ?? null;
    const relayed = relayNode ? (live.peers[net]?.[relayNode]?.endpoint ?? null) : null;
    if (relay && relayed && /:\d+$/.test(relayed)) return { via: relay, at: relayed };
    return null;
  });
  /** §6.4 header: kind and OS as text, when known. Peers carry neither yet. */
  const osText = $derived(isSelf ? (live.status?.host.os ?? null) : null);
  /** A removal in flight, in words (§7.9 step 2). */
  const removal = $derived(removalOf(net, node));

  // §6.4 Traffic.
  const ranges: readonly Range[] = ['1m', '15m', '1h'];
  const totals = $derived.by(() => {
    void live.trafficVersion;
    return windowTotals(live.traffic.get(key), RANGE_MS[range], live.lastTickMs || now);
  });
  /** Whether the window holds any samples: an empty one reads as one line, not a chart. */
  const sampled = $derived.by(() => {
    void live.trafficVersion;
    const columns = live.traffic.get(key);
    if (!columns) return false;
    const from = (live.lastTickMs || now) - RANGE_MS[range];
    for (let i = 0; i < columns.length; i += 1) {
      if ((columns.at[i] ?? 0) > from) return true;
    }
    return false;
  });
  const events = $derived(eventsOf(live.journal, net, node, exitName));

  // §6.4 Names pointing here.
  const records = $derived(live.records[net] ?? null);
  const names = $derived(peer && records ? namesPointingAt(records, peer, networkName) : []);
  let recordsAsked = false;
  $effect(() => {
    if (records !== null || recordsAsked) return;
    recordsAsked = true;
    untrack(() => actions.loadRecords(net));
  });

  // §6.4 History.
  const history = $derived(historyOf(live.journal, net, node, exitName, HISTORY_LIMIT));

  // §6.4 Actions and exits.
  const exits = $derived(live.exits[net] ?? null);
  const exitView = $derived(exits?.find((e) => e.node === node) ?? null);
  let exitsAsked = false;
  $effect(() => {
    if (exits !== null || exitsAsked) return;
    exitsAsked = true;
    untrack(() => actions.loadExits(net));
  });
  const currentExit = $derived(live.status?.exit_node ?? null);
  const usingThis = $derived(currentExit !== null && currentExit === exitName);

  /** §12.5: an action's failure as a sentence. */
  function failureText(error: ApiFailure | null): string {
    switch (error?.code) {
      case 'permission_denied':
      case 'forbidden':
        return copy.error.permissionDenied;
      case 'not_found':
        return copy.error.notFound(peer?.name ?? name);
      case 'invalid_argument':
        return copy.error.nameChars;
      case 'daemon_down':
        return copy.conn.staleActions;
      case 'os_auth_denied':
        return copy.error.osAuthDenied;
      default:
        return copy.error.internal(error?.message ?? '');
    }
  }

  /** §8.6: where a ledger op is, in words. */
  function progressText(handle: ActionHandle<unknown> | null): string | null {
    if (!handle) return null;
    switch (handle.state) {
      case 'pending':
        return copy.action.signing;
      case 'awaiting_os_auth':
        return copy.tier3.waiting;
      case 'accepted':
      case 'seen':
        if (handle.seen) return copy.action.spreading(handle.seen.by, handle.seen.of);
        return handle.slow ? copy.action.savedWaiting : copy.action.savedHere;
      default:
        return null;
    }
  }

  // §7.11 Rename.
  let editing = $state(false);
  let draft = $state('');
  let editFrom = $state('');
  let nameError = $state<string | null>(null);
  let suggestion = $state<string | null>(null);
  let spacesHint = $state(false);
  let validated = $state(false);
  let rename = $state<ActionHandle<Outcome> | null>(null);
  let renameShown = $state<ActionHandle<Outcome> | null>(null);
  let conflictFrom = $state<string | null>(null);
  let nameInput: HTMLInputElement | null = $state(null);
  let heading: HTMLElement | null = $state(null);
  let spacesTimer: ReturnType<typeof setTimeout> | null = null;

  const renameProgress = $derived(
    renameShown && !fullySeen(renameShown) ? progressText(renameShown) : null,
  );
  const renaming = $derived(rename?.state === 'pending' || rename?.state === 'awaiting_os_auth');
  /** Where the edit started (the pencil, the Actions button, the heading), for Esc. */
  let editOrigin: HTMLElement | null = null;

  async function startEdit(): Promise<void> {
    if (!peer || !admin || isStale) return;
    const active = document.activeElement;
    editOrigin = active instanceof HTMLElement && active !== document.body ? active : null;
    editing = true;
    draft = peer.name;
    editFrom = peer.name;
    nameError = null;
    suggestion = null;
    validated = false;
    conflictFrom = null;
    await tick();
    nameInput?.focus();
    nameInput?.select();
    // The Actions button sits below the fold; the field is in the header (§7.11).
    nameInput?.scrollIntoView({
      block: 'nearest',
      behavior: reducedMotion.current ? 'auto' : 'smooth',
    });
  }

  function stopEdit(): void {
    editing = false;
    nameError = null;
    suggestion = null;
    conflictFrom = null;
    const origin = editOrigin;
    editOrigin = null;
    void tick().then(() => {
      if (origin?.isConnected) origin.focus();
      else (pencilButton ?? heading)?.focus();
    });
  }

  let pencilButton: HTMLButtonElement | null = $state(null);

  $effect(() => {
    if (renameRequest.key !== key || !peer) return;
    // After the screen's own focus move, so the new field keeps the focus.
    untrack(() =>
      void tick().then(() =>
        requestAnimationFrame(() => {
          renameRequest.key = null;
          void startEdit();
        }),
      ),
    );
  });

  function input(event: Event & { currentTarget: HTMLInputElement }): void {
    const raw = event.currentTarget.value;
    const next = raw.toLowerCase().replace(/\s/g, '-');
    if (/\s/.test(raw)) {
      spacesHint = true;
      if (spacesTimer !== null) clearTimeout(spacesTimer);
      spacesTimer = setTimeout(() => (spacesHint = false), SPACES_HINT_MS);
    }
    draft = next;
    event.currentTarget.value = next;
    if (validated) void check();
  }

  function problemText(result: NameCheck): string | null {
    switch (result.problem) {
      case null:
        return null;
      case 'empty':
        return copy.error.nameEmpty;
      case 'too_long':
        return copy.error.nameTooLong;
      case 'invalid_character':
        return copy.error.nameChars;
      case 'hyphen_at_edge':
        return copy.error.nameHyphenEdge;
      case 'taken':
        return result.suggestion
          ? copy.rename.taken(result.name, networkName, result.suggestion)
          : copy.rename.taken(result.name, networkName, result.name);
    }
  }

  /** Validates the draft (§5 Input: on blur, then on every keystroke once in error). */
  async function check(): Promise<boolean> {
    const asked = draft;
    if (peer && asked === peer.name) {
      nameError = null;
      suggestion = null;
      return true;
    }
    const handle = await actions.validateName(net, asked).done;
    if (asked !== draft) return false;
    validated = true;
    if (handle.state !== 'accepted' || !handle.result) {
      nameError = failureText(handle.error);
      suggestion = null;
      return false;
    }
    nameError = problemText(handle.result);
    suggestion = handle.result.problem === 'taken' ? handle.result.suggestion : null;
    return nameError === null;
  }

  async function save(): Promise<void> {
    if (!peer || renaming || isStale) return;
    if (draft === peer.name && conflictFrom === null) {
      stopEdit();
      return;
    }
    if (!(await check())) {
      if (nameError) announce(nameError);
      return;
    }
    const from = peer.name;
    const to = draft;
    const handle = actions.renameDevice(net, node, to);
    rename = handle;
    renameShown = handle;
    await handle.done;
    if (rename !== handle) return;
    if (handle.state === 'conflict') {
      conflictFrom = editFrom;
      return;
    }
    if (handle.state === 'failed') {
      nameError = failureText(handle.error);
      announce(nameError);
      return;
    }
    rename = null;
    stopEdit();
    toast.done(copy.deviceView.renamed(from, to), {
      undo: () => {
        renameShown = actions.renameDevice(net, node, from);
      },
    });
  }

  function keydownName(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      void save();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      stopEdit();
    }
  }

  /** §7.11: Enter saves and Esc reverts; leaving the field only checks it. */
  function blurName(): void {
    if (peer && draft === peer.name) return;
    void check();
  }

  // §7.8 Use as exit: optimistic, with the outcome inline.
  let exitHandle = $state<ActionHandle<Outcome> | null>(null);
  let exitNote = $state<string | null>(null);
  async function useExit(): Promise<void> {
    if (!peer) return;
    const before = currentExit;
    exitNote = copy.exits.switching;
    const handle = actions.useExit(net, node);
    exitHandle = handle;
    await handle.done;
    if (exitHandle !== handle) return;
    exitHandle = null;
    if (handle.state === 'failed') {
      const current = before?.split('/').pop() ?? null;
      exitNote = current
        ? copy.exits.failed(peer.name, current)
        : copy.exits.failedNone(peer.name);
      return;
    }
    exitNote = copy.exits.done(peer.name);
  }

  // §13 tier 2: Allow as exit.
  let allowOpen = $state(false);
  let allowHandle = $state<ActionHandle<Outcome> | null>(null);
  async function allowExit(): Promise<void> {
    if (!peer) return;
    const device = peer.name;
    const handle = actions.grantExit(net, node, true);
    allowHandle = handle;
    await handle.done;
    allowHandle = null;
    allowOpen = false;
    if (handle.state === 'failed') {
      toast.failed(failureText(handle.error));
      return;
    }
    toast.done(copy.deviceView.allowedExit(device));
    actions.loadExits(net);
  }

  // §7.10 Ask an admin.
  let askRequest = $state<string | null>(null);
  let askOpen = $state(false);
  function askAdmin(request: string): void {
    askRequest = request;
    askOpen = true;
  }
  // A member's ask from the row's ⋯ menu opens here.
  $effect(() => {
    if (rowAsk.key !== key || rowAsk.request === null) return;
    const request = rowAsk.request;
    rowAsk.key = null;
    rowAsk.request = null;
    untrack(() => askAdmin(request));
  });
  let allowBlockedOpen = $state(false);
  const askItems: MenuItem[] = $derived.by(() => {
    if (!peer) return [];
    const items: MenuItem[] = [
      {
        label: copy.deviceView.askRemove(peer.name, networkName),
        onSelect: () => askAdmin(copy.askAdmin.request.remove(networkName, peer.name)),
      },
    ];
    items.push({
      label: copy.deviceView.askRename(peer.name),
      onSelect: () => askAdmin(copy.deviceView.renameCommand(networkName, peer.name)),
    });
    if (!exitView && !isPhone(peer)) {
      items.push({
        label: copy.deviceView.askAllowExit(peer.name),
        onSelect: () => askAdmin(copy.askAdmin.request.exitAllow(networkName, peer.name)),
      });
    }
    return items;
  });

  $effect(() => {
    if (askRenameRequest.key !== key || !peer) return;
    askRenameRequest.key = null;
    const name = peer.name;
    untrack(() => (askRequest = copy.deviceView.renameCommand(networkName, name)));
  });

  function verdictAction(): void {
    if (!verdict?.action || !peer) return;
    if (verdict.action === 'remove') {
      if (admin) setQuery({ remove: peer.name });
      else askAdmin(copy.askAdmin.request.remove(networkName, peer.name));
      return;
    }
    if (!blocked) return;
    if (admin) {
      // Straight to the rule preview, never a sheet from a sheet (§5, §12.4).
      allowBlockedOpen = true;
      return;
    }
    askAdmin(copy.askAdmin.request.allow(
      networkName,
      blocked.from_name,
      blocked.to_name,
      blocked.protocol,
      blocked.port ?? null,
    ));
  }

  const verdictActionLabel = $derived.by(() => {
    if (!verdict?.action || !peer) return null;
    if (!admin) return copy.detail.askAdmin;
    return verdict.action === 'remove' ? copy.detail.remove(networkName) : copy.verdict.allowThis;
  });

  // Copy buttons in the header and the names list.
  let copiedKey = $state<string | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | null = null;
  async function copyText(id: string, text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      return;
    }
    copiedKey = id;
    if (copiedTimer !== null) clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (copiedKey = null), COPIED_MS);
  }

  const activityHref = $derived(
    peer
      ? `${href({ name: 'activity', net: networkName || net })}?device=${encodeURIComponent(peer.name)}`
      : '',
  );

  onMount(() => {
    // Move focus into the drawer or page without drawing the ring: nothing was pressed yet.
    if (autofocus) heading?.focus({ preventScroll: true, focusVisible: false } as FocusOptions);
  });
</script>

{#snippet copyButton(id: string, text: string, label: string)}
  <button type="button" class="copy" aria-label={label} onclick={() => void copyText(id, text)}>
    {#if copiedKey === id}
      <Icon name="check" />
      <span>{copy.copyField.copied}</span>
    {:else}
      <span>{copy.copyField.copy}</span>
    {/if}
  </button>
{/snippet}

{#snippet back()}
  <IconButton icon="arrow-left" label={copy.nav.back} onclick={() => onClose?.()} />
{/snippet}

<article class="detail" aria-labelledby={titleId}>
  {#if mode !== 'page'}
    <div class="bar corner">
      <IconButton icon="x" label={copy.detail.close} onclick={() => onClose?.()} />
    </div>
  {/if}

  {#if !peer || !presence}
    <div class="title-row">
      {#if mode === 'page'}{@render back()}{/if}
      <p class="gone" id={titleId}>
        {seenBefore ? copy.error.notFound(name) : copy.error.noDevice(name, networkName)}
      </p>
    </div>
  {:else}
    <header class="head">
      {#key stamp}
        {#if stamp !== null}<span class="tint" aria-hidden="true"></span>{/if}
      {/key}
      {#if editing}
        <h2 class="sr-only" id={titleId}>{peer.display_name}</h2>
        <div class="rename">
          <Field label={copy.deviceView.nameLabel}>
            {#snippet children(control)}
              <Input
                {...control}
                bind:ref={nameInput}
                kind="name"
                value={draft}
                readonly={renaming}
                aria-invalid={nameError ? 'true' : undefined}
                aria-describedby={`${titleId}-dns ${nameError ? `${titleId}-err` : ''}`.trim()}
                oninput={input}
                onkeydown={keydownName}
                onblur={blurName}
              />
            {/snippet}
          </Field>
          {#if nameError === null}
            <p class="helper" id={`${titleId}-dns`}>
              {copy.rename.preview(draft || peer.name, networkName)}
            </p>
          {/if}
          {#if renaming}<p class="helper" role="status">{copy.action.signing}</p>{/if}
          {#if isStale}<p class="helper">{copy.conn.staleActions}</p>{/if}
          {#if spacesHint}<p class="helper">{copy.createNetwork.spaces}</p>{/if}
          {#if names.length > 0}
            <p class="helper">{copy.rename.follow(names.length, peer.name)}</p>
          {/if}
          {#if nameError}
            <p class="error" id={`${titleId}-err`}>{nameError}</p>
            {#if suggestion}
              <div>
                <Button
                  onclick={() => {
                    draft = suggestion ?? draft;
                    nameError = null;
                    suggestion = null;
                    nameInput?.focus();
                  }}
                >
                  {copy.rename.use(suggestion)}
                </Button>
              </div>
            {/if}
          {/if}
          {#if conflictFrom !== null}
            <div class="band" role="status">
              <p>
                {copy.rename.concurrent(
                  renameShown?.conflict?.resolved_by ?? '',
                  conflictFrom,
                  peer.name,
                )}
              </p>
              <div class="band-actions">
                <Button disabled={renaming} onclick={stopEdit}>{copy.rename.useTheirs}</Button>
                <Button pending={renaming} disabled={isStale} onclick={() => void save()}>
                  {copy.rename.keepMine}
                </Button>
              </div>
            </div>
          {/if}
        </div>
      {:else}
        <div class="title-row">
          {#if mode === 'page'}{@render back()}{/if}
          {#if admin && !isStale}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
            <h2
              class="t-title name editable"
              id={titleId}
              tabindex="-1"
              bind:this={heading}
              onclick={() => void startEdit()}
            >
              {peer.display_name}
            </h2>
            <span class="pencil">
              <IconButton
                bind:ref={pencilButton}
                icon="pencil"
                label={copy.palette.rename(peer.name)}
                onclick={() => void startEdit()}
              />
            </span>
          {:else}
            <h2 class="t-title name" id={titleId} tabindex="-1" bind:this={heading}>
              {peer.display_name}
            </h2>
          {/if}
          {#if isSelf}<Badge>{copy.device.you}</Badge>{/if}
        </div>
      {/if}
      {#if osText}<p class="meta">{osText}</p>{/if}
      {#if renameProgress && !editing}<p class="progress" role="status">{renameProgress}</p>{/if}
      {#if removal}<p class="progress" role="status">{removalLine(removal)}</p>{/if}
      <p class="status">
        <StatusDot
          state={presence.kind === 'waiting'
            ? 'waiting'
            : !online
              ? 'offline'
              : isStale
                ? 'stale'
                : 'online'}
        />
        <span>{online ? (isStale ? copy.status.stale : copy.status.online) : copy.status.offline}</span>
        {#if online && !isSelf}
          <PathBadge
            path={gatewayName !== null ? 'relay' : peer.path}
            relay={gatewayName ?? relay}
            flapping={isFlapping(live.journal, net, node, now)}
            endpoint={peer.endpoint}
            mtu={peer.path_mtu}
            reason={peer.path_reason}
          />
        {/if}
      </p>
      <div class="ids">
        <p>
          <code>{peer.address}</code>
          {@render copyButton('address', peer.address, copy.deviceView.copyAddress)}
        </p>
        <p>
          <code>{fqdn(peer.name, networkName)}</code>
          {@render copyButton('fqdn', fqdn(peer.name, networkName), copy.deviceView.copyName)}
        </p>
      </div>
    </header>

    {#if verdict}
      <div class="verdict" class:attention={verdict.attention}>
        <p class="t-heading verdict-text">{verdict.text}</p>
        {#if verdict.reason}
          <button
            type="button"
            class="link"
            aria-expanded={whyOpen}
            aria-controls={`${titleId}-why`}
            onclick={() => (whyOpen = !whyOpen)}
          >
            {copy.path.learnWhy}
          </button>
          <p class="reason" id={`${titleId}-why`} hidden={!whyOpen}>{verdict.reason}</p>
        {/if}
        {#if verdictActionLabel}
          <div>
            <Button
              variant={admin && verdict.action === 'remove' ? 'danger-quiet' : 'secondary'}
              disabled={isStale}
              onclick={verdictAction}
            >
              {verdictActionLabel}
            </Button>
          </div>
        {/if}
      </div>
    {/if}

    {#if hops.length > 0}
      <section class="section">
        <h3 class="t-heading">{copy.detail.path}</h3>
        <div
          class="diagram"
          class:with-rtt={rttMs !== null}
          role="img"
          aria-label={copy.deviceView.pathLabel(
            [selfLabel, ...hops.map((h) => h.label)],
            rttMs === null ? null : copy.spoken.milliseconds(Math.round(rttMs)),
          )}
        >
          <span class="node"><span class="pin"></span><span class="who" title={selfLabel}>{selfLabel}</span></span>
          {#each hops as hop, i (i)}
            <!-- The end-to-end latency sits on the first edge, where the path starts. -->
            <span class="edge {hop.style}">
              {#if i === 0 && rttMs !== null}
                <span class="edge-rtt">{formatRtt(isStale ? null : rttMs)}</span>
              {/if}
            </span>
            <span class="node"><span class="pin"></span><span class="who" title={who(hop.label)}>{who(hop.label)}</span></span>
          {/each}
        </div>
        {#if endpoint || peer.path_mtu !== null}
          <p class="path-facts">
            {#if endpoint}
              <span class="label">{copy.deviceView.endpoint}</span>
              {#if endpoint.via}<span>{endpoint.via}</span>{/if}
              <code>{endpoint.at}</code>
            {/if}
            {#if peer.path_mtu !== null}<span>{copy.path.mtu(peer.path_mtu)}</span>{/if}
          </p>
        {/if}
        <!-- `Learn why` already says it for a relayed path. -->
        {#if peer.path_reason && !verdict?.reason}
          <p class="reason">{sentence(pathReason(peer.path_reason))}</p>
        {/if}
        {#if reporter}
          <p class="meta">
            {copy.path.reportedBy(reporter.who, agoMs(reporter.link.age_secs * 1000))}
          </p>
        {/if}
      </section>
    {/if}

    {#if !isPhone(peer)}
      <DeviceSettings {net} {node} name={peer.name} {online} {isSelf} stale={isStale} />
    {/if}

    {#if !isSelf}
      <section class="section">
        <div class="section-head">
          <h3 class="t-heading">{copy.detail.traffic}</h3>
          <Segmented
            value={range}
            label={copy.chart.rangeLabel}
            options={ranges.map((r) => ({
              value: r,
              label: copy.chart.ranges[r],
              ariaLabel: copy.chart.rangeNames[r],
            }))}
            onValueChange={(next) => setQuery({ range: next === '1h' ? null : next, at: null }, { replace: true })}
          />
        </div>
        {#if !sampled && !online}
          <p class="meta">{copy.deviceView.noTraffic[range]}</p>
        {:else}
          <!-- The totals sit by the chart, above its `Show data` table. -->
          <p class="meta">{copy.deviceView.totals[range](bytes(totals.rx), bytes(totals.tx))}</p>
          <Series peer={key} {range} {events} {at} />
        {/if}
        <TrafficDays network={net} {node} />
      </section>
    {/if}

    {#if names.length > 0}
      <section class="section">
        <h3 class="t-heading">{copy.detail.names}</h3>
        <ul class="names">
          {#each names as record (record.name)}
            <li>
              <code>{record.name}</code>
              {@render copyButton(
                `name:${record.name}`,
                record.name,
                copy.deviceView.copyThis(record.name),
              )}
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="section">
      <h3 class="t-heading">{copy.detail.history}</h3>
      {#if history.length === 0}
        <p class="meta">{copy.deviceView.noHistory}</p>
      {:else}
        <div class="history">
          {#each history as entry (entry.seq)}
            <EventRow {entry} />
          {/each}
        </div>
        <a
          class="link"
          href={activityHref}
          onclick={(event) => {
            event.preventDefault();
            go(activityHref);
          }}
        >
          {copy.detail.allHistory}
        </a>
      {/if}
    </section>

    <section class="section actions">
      <div class="action-row">
        {#if admin}
          <Button disabled={isStale} onclick={() => void startEdit()}>{copy.detail.rename}</Button>
        {/if}
        {#if !isSelf && exitView?.allowed && !usingThis}
          <Button disabled={isStale} pending={exitHandle !== null} onclick={() => void useExit()}>
            {copy.detail.useAsExit}
          </Button>
        {:else if !isSelf && admin && !exitView && !isPhone(peer)}
          <Button disabled={isStale} onclick={() => (allowOpen = true)}>
            {copy.detail.allowAsExit}
          </Button>
        {/if}
        {#if !admin && !isSelf}
          <DropdownMenu items={askItems}>
            {#snippet trigger(props)}
              <Button {...props}>{copy.detail.askAdmin}</Button>
            {/snippet}
          </DropdownMenu>
        {/if}
      </div>
      {#if exitNote}<p class="meta" role="status">{exitNote}</p>{/if}
      {#if usingThis && !exitNote}<p class="meta">{copy.deviceView.usingExit(peer.name)}</p>{/if}
      {#if isSelf}
        <div class="danger-row">
          <Button variant="danger-quiet" disabled={isStale} onclick={() => setQuery({ remove: peer?.name ?? null })}>
            {copy.detail.leave(networkName)}
          </Button>
        </div>
      {:else if admin}
        <div class="danger-row">
          <Button variant="danger-quiet" disabled={isStale} onclick={() => setQuery({ remove: peer?.name ?? null })}>
            {copy.detail.remove(networkName)}
          </Button>
        </div>
      {/if}
      {#if isStale}<p class="meta">{copy.conn.staleActions}</p>{/if}
    </section>

    <AlertDialog
      bind:open={allowOpen}
      title={copy.deviceView.allowExitTitle(peer.name)}
      description={copy.deviceView.allowExitBody(peer.name, networkName)}
      cancelLabel={copy.action.cancel}
      confirmLabel={copy.deviceView.allowExitSubmit(peer.name)}
      confirmVariant="primary"
      pending={allowHandle !== null}
      onConfirm={() => void allowExit()}
    />
  {/if}
</article>

<span class="sr-only" aria-live="polite">{copiedKey ? copy.copyField.copied : ''}</span>

{#if askRequest !== null}
  <AskAdminSheet {net} request={askRequest} bind:open={askOpen} />
{/if}
{#if admin && blocked}
  <AllowBlocked bind:open={allowBlockedOpen} id={blocked.id} />
{/if}

<style>
  .detail {
    display: grid;
    align-content: start;
    gap: var(--s-6);
    padding: var(--s-4) var(--s-5) var(--s-7);
  }
  .detail {
    position: relative;
  }
  .bar.corner {
    position: absolute;
    top: var(--s-4);
    right: var(--s-4);
    z-index: 1;
  }
  /* The header's rows, the rename field included, stop short of the close button. */
  .bar.corner ~ .head .title-row,
  .bar.corner ~ .head .rename,
  .bar.corner ~ .title-row {
    padding-right: var(--s-7);
  }
  .gone {
    color: var(--ink-2);
  }
  .head {
    position: relative;
    display: grid;
    gap: var(--s-2);
    border-radius: var(--r-2);
  }
  .tint {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    pointer-events: none;
    background: var(--changed);
    opacity: 0;
    animation: detail-tint var(--d-tint) var(--e-linear);
  }
  @keyframes detail-tint {
    from {
      opacity: 1;
    }
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .name {
    min-width: 0;
    overflow-wrap: anywhere;
    outline: none;
  }
  .name:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
    border-radius: var(--r-1);
  }
  .editable {
    cursor: text;
  }
  .pencil {
    opacity: 0;
    transition: opacity var(--d-hover) var(--e-linear);
  }
  .title-row:hover .pencil,
  .pencil:focus-within {
    opacity: 1;
  }
  @media (pointer: coarse) {
    .pencil {
      opacity: 1;
    }
  }
  .rename {
    display: grid;
    gap: var(--s-1);
  }
  .helper {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .error {
    font: var(--t-small);
    color: var(--danger);
  }
  .band {
    display: grid;
    gap: var(--s-2);
    padding: var(--s-3);
    border-radius: var(--r-2);
    background: var(--attn-wash);
    color: var(--attn-text);
  }
  .band-actions {
    display: flex;
    gap: var(--s-3);
  }
  .progress {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .status {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-2);
    color: var(--ink-2);
  }
  .status :global(.badge) {
    margin-left: var(--s-2);
  }
  .ids {
    display: grid;
    gap: var(--s-1);
  }
  .ids p,
  .names li {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  code {
    font: var(--t-mono);
    color: var(--ink-1);
    overflow-wrap: anywhere;
  }
  .copy {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: var(--target-fine);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    font: var(--t-small);
    font-weight: 600;
    color: var(--ink-2);
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .copy:hover {
    background: var(--sunken);
    color: var(--ink-1);
  }
  @media (pointer: coarse) {
    .copy {
      height: var(--control);
    }
  }
  .verdict {
    display: grid;
    justify-items: start;
    gap: var(--s-2);
  }
  .verdict-text {
    max-width: var(--measure);
  }
  /* A fault the person can act on carries the Needs-you treatment, like its row. */
  .verdict.attention {
    position: relative;
    padding: var(--s-3) var(--s-3) var(--s-3) var(--s-4);
    border-radius: var(--r-2);
    background: var(--attn-wash);
  }
  .verdict.attention::before {
    content: '';
    position: absolute;
    inset-block: 0;
    left: 0;
    width: var(--lead-bar);
    background: var(--attn);
  }
  .reason {
    max-width: var(--measure);
    color: var(--ink-2);
  }
  .link {
    display: inline-flex;
    align-items: center;
    min-height: var(--s-5);
    justify-self: start;
    color: var(--ink-1);
    text-decoration: underline;
    text-decoration-color: var(--line-strong);
    text-underline-offset: var(--s-1);
    cursor: pointer;
    transition: text-decoration-color var(--d-hover) var(--e-linear);
  }
  .link:hover {
    text-decoration-color: currentColor;
  }
  @media (pointer: coarse) {
    .link {
      min-height: var(--target-touch);
    }
  }
  .section {
    display: grid;
    align-content: start;
    gap: var(--s-3);
  }
  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    flex-wrap: wrap;
  }
  .meta {
    font: var(--t-small);
    color: var(--ink-3);
  }
  /* Dots joined edge to edge; each label hangs under its dot, the ends flush with the row. */
  .diagram {
    position: relative;
    display: flex;
    align-items: flex-start;
    padding-bottom: var(--s-5);
  }
  .diagram.with-rtt {
    padding-top: var(--s-5);
  }
  .node {
    position: relative;
    flex: none;
  }
  .who {
    position: absolute;
    top: calc(var(--dot) + var(--s-1));
    left: 50%;
    translate: -50% 0;
    max-width: calc(var(--s-8) * 2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--t-small);
    color: var(--ink-2);
  }
  .node:first-child .who {
    left: 0;
    translate: none;
  }
  .node:last-child .who {
    left: auto;
    right: 0;
    translate: none;
  }
  .pin {
    display: block;
    width: var(--dot);
    height: var(--dot);
    border-radius: var(--r-full);
    background: var(--ink-1);
  }
  .edge {
    position: relative;
    flex: 1;
    min-width: var(--s-6);
    margin-top: calc((var(--dot) - var(--dot-ring)) / 2);
    border-top: var(--dot-ring) solid var(--ink-2);
  }
  .edge.relayed {
    border-top-style: dashed;
  }
  .edge.gateway {
    border-top-style: dotted;
  }
  .edge-rtt {
    position: absolute;
    bottom: calc(100% + var(--s-1));
    left: 50%;
    translate: -50% 0;
    text-align: center;
    font: var(--t-small);
    color: var(--ink-1);
    white-space: nowrap;
  }
  .path-facts {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s-1) var(--s-3);
  }
  .path-facts .label {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .names {
    display: grid;
    gap: var(--s-1);
  }
  .history {
    display: grid;
  }
  .actions {
    gap: 0;
  }
  .action-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3);
  }
  .actions .meta {
    margin-top: var(--s-2);
  }
  .danger-row {
    margin-top: var(--s-5);
  }
  @media (max-width: 40rem) {
    .detail {
      padding-inline: var(--gutter-mobile);
    }
  }
</style>
