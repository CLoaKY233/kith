<!--
@component
NeedsYouCard (spec §5, §7.3, §7.5): one card for the Overview strip, the header popover,
the AddDevice sheet and `?need=`. `--surface`, hairline, a 2 px `--attn` leading bar.
  - join: who asks, what it says (a claim), what this device saw, the check code, a name
    field, then [Deny] [Approve]. Approve signs, then reads `Signed. Spreading · n of m
    devices` (or `Saved on this device`), then `dev-box is online · direct · 14 ms` with
    `Show device`. Deny is instant with Undo for 8 s; the deny is sent when Undo lapses.
    A 409 shows who won (`Approved by laptop · just now`) and removes the buttons.
    Expired requests say so. `A` / `D` approve or deny while focus is inside the card.
  - blocked: `phone → nas :22 (ssh) · blocked 4× in the last 10 min` with [Ignore] and
    [Allow this…] (admin) or [Ask an admin] (member).
  - health: a network problem and its fix, with `Show device`, the command to copy or a
    link to This device; or a daemon warning with Dismiss.
While the connection is stale, actions are aria-disabled with the reason inline.
When an action removes the focused control, focus moves to what replaces it, else to the
next card, else to the list's `[data-needs-heading]`, else to the main region.
Props:
  - need: Need. The card finds the request, group or problem itself (frame/needs).
  - primary?: boolean (default true). Approve is the primary button; pass false where
    another card or action already holds the view's one primary (lists make their first
    join card primary).
  - layered?: boolean. The card sits in a popover: `Allow this…` opens `?need=` instead
    of a popover of its own (no layer opens from another, §5). Inside a Sheet this is
    detected by itself.
  - twinBand?: boolean (default true). Shows the `Two devices asked…` band; AddDevice
    shows one band for all its cards instead.
  - onDone?: () => void. Called when the card has nothing left to show (approved device
    online and the person moved on, deny sent, outcome dismissed); `?need=` closes on it.
-->
<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { connection } from '../../frame/conn.svelte';
  import { layers } from '../../frame/layers.svelte';
  import { deviceName, isAdmin, networkName, peers } from '../../frame/net';
  import { needs, type NeedDetail } from '../../frame/needs.svelte';
  import { prefs } from '../../frame/prefs.svelte';
  import { follow, go, href, openOverlay } from '../../router';
  import AllowBlocked, { allowRequest } from '../../sheets/AllowBlocked.svelte';
  import { copy, DOT } from '../copy';
  import { relative, rtt } from '../format';
  import Icon from '../icons/Icon.svelte';
  import { tintKey } from '../live/keys';
  import { actions, live } from '../live/store.svelte';
  import {
    RELATIVE_TIME_REFRESH_MS,
    SHEET_CLOSE_AFTER_ONLINE_MS,
    UNDO_MS,
  } from '../live/timing';
  import type { ActionHandle, Need } from '../types';
  import type { NameCheck, Outcome } from '../types.gen';
  import Button from '../ui/Button.svelte';
  import CopyField from '../ui/CopyField.svelte';
  import Field from '../ui/Field.svelte';
  import Input from '../ui/Input.svelte';
  import { insideModal, narrow } from '../ui/layers.svelte';
  import { toast } from '../ui/Toast.svelte';
  import AskAdminSheet from './AskAdminSheet.svelte';
  import CheckCode from './CheckCode.svelte';
  import { explain, nameProblem } from './errors';
  import { problemTarget } from './headline';

  let {
    need,
    primary = true,
    layered = false,
    twinBand = true,
    onDone,
  }: {
    need: Need;
    primary?: boolean;
    layered?: boolean;
    twinBand?: boolean;
    onDone?: () => void;
  } = $props();

  const titleId = $props.id();
  const inLayer = insideModal();

  // The last thing known about this need, so an outcome stays readable after the
  // request leaves `live.pending`.
  let known = $state.raw<NeedDetail | null>(null);
  $effect(() => {
    const found = needs.find(need.id);
    if (found) known = found;
  });
  const detail = $derived(needs.find(need.id) ?? known);

  let now = $state(live.now());
  $effect(() => {
    const timer = setInterval(() => (now = live.now()), RELATIVE_TIME_REFRESH_MS);
    return () => clearInterval(timer);
  });

  const frozen = $derived(connection.frozen);
  const admin = $derived(detail ? isAdmin(detail.network) : false);
  const network = $derived(detail ? networkName(detail.network) : '');

  /** A new item carries the 4 s change tint (§8.4). */
  const stamp = $derived(live.changed.get(tintKey.need(need.id)));

  // Focus ----------------------------------------------------------------------------

  let card = $state<HTMLElement | null>(null);

  /** Focuses what `target` returns once the DOM has caught up, if focus was in this card. */
  async function refocus(target: () => HTMLElement | null | undefined): Promise<void> {
    const had = card?.contains(document.activeElement) ?? false;
    await tick();
    if (had) target()?.focus();
  }

  /**
   * The card is about to leave: focus moves on to the next card, else the list's heading,
   * else the main region, so a keyboard user keeps their place (§10).
   */
  function handOff(): void {
    const self = card;
    if (!self?.contains(document.activeElement)) return;
    const siblings = [...(self.parentElement?.children ?? [])].filter(
      (el): el is HTMLElement => el instanceof HTMLElement && el !== self && el.matches('.card'),
    );
    const after = siblings.find(
      (el) => (self.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0,
    );
    const next = after ?? siblings.at(-1);
    const control = next?.querySelector<HTMLElement>(
      'button:not([aria-disabled]), input, a[href]',
    );
    if (control) {
      control.focus();
      return;
    }
    const heading = self
      .closest('[data-needs]')
      ?.querySelector<HTMLElement>('[data-needs-heading]');
    void tick().then(() => {
      if (heading?.isConnected) heading.focus();
      else document.querySelector<HTMLElement>('main')?.focus();
    });
  }

  // Join ---------------------------------------------------------------------------

  const join = $derived(detail?.kind === 'join' ? detail.join : null);
  let name = $state('');
  let nameSeeded = false;
  $effect(() => {
    if (join && !nameSeeded) {
      name = join.claimed.name;
      nameSeeded = true;
    }
  });
  let nameError = $state<string | null>(null);
  let suggestion = $state<string | null>(null);
  let check: ActionHandle<NameCheck> | null = null;

  async function validate(): Promise<void> {
    if (!join) return;
    const handle = actions.validateName(join.network, name);
    check = handle;
    await handle.done;
    if (check !== handle) return;
    const result = handle.result;
    if (!result) return;
    suggestion = result.problem === 'taken' ? result.suggestion : null;
    nameError =
      result.problem === null
        ? null
        : result.problem === 'taken' && result.suggestion
          ? copy.rename.taken(result.name, network, result.suggestion)
          : nameProblem(result.problem);
  }

  /** §7.1: names are lower case, with dashes for spaces. */
  function typed(event: Event & { currentTarget: HTMLInputElement }): void {
    name = event.currentTarget.value.toLowerCase().replace(/ /g, '-');
    event.currentTarget.value = name;
    if (nameError !== null) void validate();
  }

  function useSuggestion(): void {
    if (suggestion === null) return;
    name = suggestion;
    nameError = null;
    suggestion = null;
  }

  const twin = $derived(
    join
      ? live.pending.joins.some(
          (j) => j.id !== join.id && j.network === join.network && j.claimed.name === join.claimed.name,
        )
      : false,
  );
  const expired = $derived(join !== null && now >= join.expires_at_ms);

  let approval = $state.raw<ActionHandle<Outcome> | null>(null);
  let denial = $state.raw<ActionHandle<Outcome> | null>(null);
  /** Deny waits out its Undo window before anything is sent. */
  let denyTimer: ReturnType<typeof setTimeout> | null = null;
  let deniedAt = 0;
  let denying = $state(false);
  let failure = $state<string | null>(null);
  let checking = $state(false);

  const settled = $derived(approval ?? denial);
  const conflict = $derived(settled?.state === 'conflict' ? settled.conflict : null);
  /** While signing, the form stays and Approve shows pending (§5, §3.5). */
  const signing = $derived(
    approval !== null && (approval.state === 'pending' || approval.state === 'awaiting_os_auth'),
  );

  let undoButton = $state<HTMLButtonElement | null>(null);
  let denyButton = $state<HTMLButtonElement | null>(null);
  let outcomeLine = $state<HTMLElement | null>(null);
  let showButton = $state<HTMLButtonElement | null>(null);

  async function approve(): Promise<void> {
    if (!join || frozen || approval || denying || expired || checking) return;
    if (name.trim() === '') {
      nameError = copy.error.nameEmpty;
      return;
    }
    checking = true;
    await validate();
    checking = false;
    if (nameError !== null || !join || approval || denying) return;
    if (detail) needs.hold(detail);
    failure = null;
    const handle = actions.approveJoin(join.network, join.node, name);
    approval = handle;
    void handle.done.then((h) => {
      if (h.state === 'failed' && h.error) {
        failure = explain(h.error, join.claimed.name);
        approval = null;
        needs.release(need.id);
      }
    });
  }

  function sendDeny(): void {
    denyTimer = null;
    if (!join) return;
    const handle = actions.denyJoin(join.network, join.node);
    denial = handle;
    void handle.done.then((h) => {
      if (h.state === 'failed' && h.error) {
        failure = explain(h.error, join.claimed.name);
        denying = false;
        denial = null;
        needs.release(need.id);
        return;
      }
      if (h.state !== 'conflict') finish();
    });
  }

  function deny(): void {
    if (!join || frozen || approval || denying || expired) return;
    if (detail) needs.hold(detail);
    failure = null;
    denying = true;
    deniedAt = Date.now();
    denyTimer = setTimeout(sendDeny, UNDO_MS);
    void refocus(() => undoButton);
  }

  function undoDeny(): void {
    if (denyTimer !== null) clearTimeout(denyTimer);
    denyTimer = null;
    denying = false;
    needs.release(need.id);
    void refocus(() => denyButton);
  }

  onDestroy(() => {
    // Closing the sheet or leaving the page keeps the deny and its Undo: a toast holds the
    // rest of the window, and the deny is sent when it lapses (§7.3 step 6).
    if (denyTimer === null || !join) return;
    clearTimeout(denyTimer);
    const { network: net, node, id } = join;
    const left = Math.max(0, deniedAt + UNDO_MS - Date.now());
    prefs.ignore(id, live.now());
    needs.release(id);
    const timer = setTimeout(() => {
      void actions.denyJoin(net, node).done.then((h) => {
        if (h.state === 'failed') prefs.unignore(id);
      });
    }, left);
    toast.done(copy.join.denied(join.claimed.name), {
      undoMs: left,
      undo: () => {
        clearTimeout(timer);
        prefs.unignore(id);
      },
    });
  });

  const joined = $derived(
    join && approval?.state !== 'failed' && approval?.state !== 'conflict'
      ? (live.peers[join.network]?.[join.node] ?? null)
      : null,
  );
  const joinedOnline = $derived(approval !== null && joined?.online === true);
  const joinedNumbers = $derived(join ? live.numbers[`${join.network}/${join.node}`] : undefined);

  // After the person's own Approve, focus follows the outcome, then `Show device`. §11.2's
  // "live data never moves focus" is about data, not their action.
  let outcomeShown = false;
  $effect(() => {
    if (!approval || signing || outcomeShown) return;
    outcomeShown = true;
    void refocus(() => outcomeLine);
  });
  let showFocused = false;
  $effect(() => {
    if (!joinedOnline || showFocused || !showButton) return;
    showFocused = true;
    if (card?.contains(document.activeElement)) showButton.focus();
  });

  function pathText(): string {
    const path = joinedNumbers?.path ?? joined?.path ?? null;
    const relay = joinedNumbers?.relay ? deviceName(join?.network ?? '', joinedNumbers.relay) : '';
    if (path === 'relay') return copy.path.via(relay);
    if (path === 'stealth') return copy.path.stealthVia(relay);
    return copy.path.direct;
  }

  function progress(handle: ActionHandle<Outcome>): string {
    if (handle.seen) return copy.action.spreading(handle.seen.by, handle.seen.of);
    if (handle.slow) return copy.action.savedWaiting;
    return copy.action.savedHere;
  }

  /** §7.3 step 7: `Approved by laptop · just now`. */
  function when(atMs: number): string {
    return now - atMs < 60_000 ? copy.join.justNow : relative(atMs, now);
  }

  // Close after the device is online, unless the person is in the card (§7.3 step 5).
  let inside = $state(false);
  $effect(() => {
    if (!joinedOnline || inside) return;
    const timer = setTimeout(finish, SHEET_CLOSE_AFTER_ONLINE_MS);
    return () => clearTimeout(timer);
  });

  function track(): void {
    inside =
      card !== null && (card.matches(':hover') || card.contains(document.activeElement));
  }

  /**
   * The request is settled here: release the held card and keep it hidden even while
   * `live.pending` still lists it (until the next refresh drops it).
   */
  function finish(): void {
    handOff();
    prefs.ignore(need.id, live.now());
    needs.release(need.id);
    onDone?.();
  }

  function showDevice(): void {
    if (!join) return;
    finish();
    go(href({ name: 'device', net: join.network, dev: join.node }));
  }

  function keys(event: KeyboardEvent): void {
    if (!join || !prefs.shortcuts || event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable]')) return;
    const key = event.key.toLowerCase();
    if (key === 'a') {
      event.preventDefault();
      void approve();
    } else if (key === 'd') {
      event.preventDefault();
      deny();
    }
  }

  /** The card delegates keys and tracks the pointer for the controls inside it. */
  function listen(node: HTMLElement): () => void {
    const leave = (): void => {
      inside = false;
    };
    const later = (): void => queueMicrotask(track);
    node.addEventListener('keydown', keys);
    node.addEventListener('pointerenter', track);
    node.addEventListener('pointerleave', leave);
    node.addEventListener('focusin', track);
    node.addEventListener('focusout', later);
    return () => {
      node.removeEventListener('keydown', keys);
      node.removeEventListener('pointerenter', track);
      node.removeEventListener('pointerleave', leave);
      node.removeEventListener('focusin', track);
      node.removeEventListener('focusout', later);
    };
  }

  // Blocked ------------------------------------------------------------------------

  const blocked = $derived(detail?.kind === 'blocked' ? detail.blocked : null);
  let allowOpen = $state(false);
  let asking = $state(false);
  /** Desktop, outside any layer: the rule opens in a popover anchored here (§7.5 step 2). */
  const popover = $derived(!narrow.current && !layered && !inLayer);

  /** Mobile, or inside a layer: the Allow sheet is `?need=<id>`, so Back closes it. */
  function openAllow(): void {
    if (!blocked || frozen) return;
    layers.needs = false;
    openOverlay('need', blocked.id);
  }

  function ignore(): void {
    if (!blocked) return;
    const id = blocked.id;
    handOff();
    prefs.ignore(id, live.now());
    toast.done(copy.blocked.ignored, { undo: () => prefs.unignore(id) });
    onDone?.();
  }

  // Health -------------------------------------------------------------------------

  const health = $derived(detail?.kind === 'health' ? detail : null);
  const target = $derived(
    health?.fix ? problemTarget({ what: health.what, fix: health.fix }, peers(health.network)) : null,
  );
  const deviceHref = $derived(
    health && target?.kind === 'device'
      ? href({ name: 'device', net: health.network, dev: target.node })
      : null,
  );
  const thisHref = href({ name: 'this' });

  function dismissWarning(): void {
    if (!health) return;
    handOff();
    prefs.ignore(health.id, live.now());
    onDone?.();
  }

  function leaveTo(event: MouseEvent, to: string): void {
    onDone?.();
    follow(event, to);
  }
</script>

{#if detail}
  <article
    bind:this={card}
    class="card"
    class:compact={!join}
    class:changed={stamp !== undefined}
    aria-labelledby={titleId}
    tabindex="-1"
    {@attach listen}
  >
    {#if join}
      <h3 id={titleId} class="title">{copy.join.wants(join.claimed.name, network)}</h3>
      {#if twinBand && twin && !settled}
        <p class="band">{copy.join.twoSameName(join.claimed.name)}</p>
      {/if}
      <dl class="facts">
        {#if join.claimed.os}
          <div><dt>{copy.join.says}</dt><dd>{join.claimed.os}</dd></div>
        {/if}
        <div>
          <dt>{copy.join.seen}</dt>
          <dd>
            {#if join.seen.ip}{copy.join.seenFrom(join.seen.ip, join.seen.key_fpr)}{:else}{copy.join.seenKey(join.seen.key_fpr)}{/if}
          </dd>
        </div>
        <div class="code">
          <dt>{copy.join.codeLabel}</dt>
          <dd><CheckCode code={join.check_code} /></dd>
        </div>
      </dl>

      {#if conflict}
        <div class="row">
          <p class="status" role="status">
            {conflict.resolution === 'denied'
              ? copy.join.deniedBy(deviceName(join.network, conflict.resolved_by), when(conflict.at_ms))
              : copy.join.lostRace(deviceName(join.network, conflict.resolved_by), when(conflict.at_ms))}
          </p>
          <Button variant="ghost" onclick={finish}>{copy.action.dismiss}</Button>
        </div>
      {:else if expired && !approval}
        <div class="row">
          <p class="status" role="status">{copy.join.expired(join.claimed.name)}</p>
          <Button variant="ghost" onclick={finish}>{copy.action.dismiss}</Button>
        </div>
      {:else if approval && !signing}
        <p class="outcome" role="status" tabindex="-1" bind:this={outcomeLine}>
          {#if joinedOnline && joined}
            {copy.join.online(joined.name, pathText(), rtt(joinedNumbers?.rttMs ?? null))}
          {:else}
            {progress(approval)}
          {/if}
        </p>
        {#if joinedOnline}
          <div class="actions">
            <Button bind:ref={showButton} onclick={showDevice}>{copy.join.showDevice}</Button>
          </div>
        {/if}
      {:else if denying}
        <p class="outcome" role="status">
          <span>{copy.join.denied(join.claimed.name)}</span>
          {#if !denial}
            <span class="dot" aria-hidden="true">{DOT.trim()}</span>
            <Button variant="ghost" bind:ref={undoButton} onclick={undoDeny}>{copy.action.undo}</Button>
          {/if}
        </p>
      {:else}
        <p class="hint">{copy.join.approveOnlyIf}</p>
        <div class="name">
          <Field label={copy.join.name} error={nameError}>
            {#snippet children(control)}
              <Input
                {...control}
                kind="name"
                value={name}
                oninput={typed}
                onblur={() => void validate()}
                onkeydown={(e) => e.key === 'Enter' && void approve()}
              />
            {/snippet}
          </Field>
        </div>
        {#if suggestion}
          <div><Button variant="ghost" onclick={useSuggestion}>{copy.rename.use(suggestion)}</Button></div>
        {/if}
        {#if failure}<p class="error" role="alert">{failure}</p>{/if}
        <div class="actions">
          <Button bind:ref={denyButton} disabled={frozen || signing} onclick={deny}>{copy.join.deny}</Button>
          <Button
            variant={primary ? 'primary' : 'secondary'}
            disabled={frozen}
            pending={signing || checking}
            pendingLabel={approval?.state === 'awaiting_os_auth' ? copy.action.awaitingOsAuth : undefined}
            onclick={() => void approve()}
          >
            {copy.join.approve}
          </Button>
        </div>
        {#if frozen}<p class="reason">{copy.conn.staleActions}</p>{/if}
      {/if}
    {:else if blocked}
      <p id={titleId} class="line">
        <Icon name="ban" />
        <span>
          {copy.blocked.card(
            blocked.from_name,
            blocked.to_name,
            blocked.port === null ? '' : copy.blocked.port(blocked.port, copy.services[blocked.port] ?? null),
            blocked.count,
          )}
        </span>
      </p>
      <div class="actions">
        <Button variant="ghost" onclick={ignore}>{copy.blocked.ignore}</Button>
        {#if !admin}
          <Button aria-expanded={asking} onclick={() => (asking = !asking)}>{copy.exits.askAdmin}</Button>
        {:else if popover}
          <AllowBlocked bind:open={allowOpen} id={blocked.id}>
            {#snippet trigger(props)}
              <Button {...props} disabled={frozen}>{copy.blocked.allowThis}</Button>
            {/snippet}
          </AllowBlocked>
        {:else}
          <Button aria-haspopup="dialog" disabled={frozen} onclick={openAllow}>
            {copy.blocked.allowThis}
          </Button>
        {/if}
      </div>
      {#if frozen && admin}<p class="reason wide">{copy.conn.staleActions}</p>{/if}
      {#if asking && !admin}
        <div class="wide">
          <AskAdminSheet bare net={blocked.network} request={allowRequest(blocked, network)} />
        </div>
      {/if}
    {:else if health}
      <div class="text">
        <p id={titleId} class="line">
          <Icon name={health.fix === null ? 'triangle-alert' : 'circle-x'} />
          <span>{health.what}</span>
        </p>
        {#if health.fix}<p class="hint">{health.fix}</p>{/if}
        {#if health.atMs !== null}
          <time class="meta" datetime={new Date(health.atMs).toISOString()}>{relative(health.atMs, now)}</time>
        {/if}
      </div>
      {#if health.atMs !== null}
        <div class="actions">
          <Button variant="ghost" onclick={dismissWarning}>{copy.action.dismiss}</Button>
        </div>
      {:else if deviceHref}
        <div class="actions">
          <a class="link" href={deviceHref} onclick={(e) => leaveTo(e, deviceHref)}>{copy.join.showDevice}</a>
        </div>
      {:else if target?.kind === 'this'}
        <div class="actions">
          <a class="link" href={thisHref} onclick={(e) => leaveTo(e, thisHref)}>{copy.nav.thisDevice}</a>
        </div>
      {:else if target?.kind === 'command'}
        <div class="wide">
          <CopyField label={copy.gate.commandLabel} value={target.command} />
        </div>
      {/if}
    {/if}
  </article>
{/if}

<style>
  .card {
    position: relative;
    display: grid;
    gap: var(--s-3);
    padding: var(--s-4);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface);
    outline-offset: var(--s-1);
  }
  /* The amber mark sits inside the left edge, clear of the rounded corners. */
  .card::before {
    content: '';
    position: absolute;
    inset-block: var(--s-4);
    left: calc(-1 * var(--hairline));
    width: var(--lead-bar);
    border-radius: var(--r-full);
    background: var(--attn);
  }
  .card.changed {
    animation: need-tint var(--d-tint) var(--e-linear);
  }
  @keyframes need-tint {
    from {
      background: var(--changed);
    }
  }
  /* Blocked and warning cards: sentence on the left, actions on the right (≥40rem). */
  @media (width >= 40rem) {
    .card.compact {
      grid-template-columns: minmax(0, 1fr) auto;
      align-items: center;
      column-gap: var(--s-4);
    }
    .card.compact .wide {
      grid-column: 1 / -1;
    }
  }
  .title {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
  }
  .band {
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-2);
    background: var(--attn-wash);
    color: var(--attn-text);
  }
  .facts {
    display: grid;
    gap: var(--s-1);
  }
  .facts div {
    display: flex;
    align-items: baseline;
    gap: var(--s-2);
  }
  .facts dt {
    color: var(--ink-3);
  }
  .facts .code {
    align-items: center;
    gap: var(--s-3);
    margin-top: var(--s-2);
  }
  .text {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  .hint {
    color: var(--ink-2);
    max-width: var(--measure);
  }
  .name {
    max-width: 360px;
  }
  .line {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
  }
  .line :global(.icon) {
    margin-top: var(--s-1);
    color: var(--ink-3);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-items: center;
    gap: var(--s-3);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: var(--s-3);
  }
  .status {
    color: var(--ink-2);
  }
  .outcome {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--control);
    color: var(--ink-2);
    outline-offset: var(--s-1);
  }
  .dot {
    color: var(--ink-3);
  }
  .link {
    color: var(--ink-2);
    text-decoration: underline;
    text-decoration-thickness: var(--hairline);
    text-underline-offset: var(--s-1);
    border-radius: var(--r-1);
    transition: color var(--d-hover) var(--e-linear);
  }
  .link:hover {
    color: var(--ink-1);
  }
  .error {
    color: var(--danger);
  }
  .reason,
  .meta {
    font: var(--t-small);
    color: var(--ink-3);
  }
</style>
