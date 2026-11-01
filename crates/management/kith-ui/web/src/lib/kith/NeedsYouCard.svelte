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
  - health: a network problem and its fix, or a daemon warning with Dismiss.
While the connection is stale, actions are aria-disabled with the reason inline.
Props:
  - need: Need. The card finds the request, group or problem itself (frame/needs).
  - primary?: boolean (default true). Approve is the primary button; pass false where
    another card or action already holds the view's one primary.
  - onDone?: () => void. Called when the card has nothing left to show (approved device
    online and the person moved on, deny sent, outcome dismissed); `?need=` closes on it.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { connection } from '../../frame/conn.svelte';
  import { deviceName, isAdmin, networkName } from '../../frame/net';
  import { needs, type NeedDetail } from '../../frame/needs.svelte';
  import { prefs } from '../../frame/prefs.svelte';
  import { go, href } from '../../router';
  import AllowBlocked from '../../sheets/AllowBlocked.svelte';
  import { copy } from '../copy';
  import { relative, rtt } from '../format';
  import Icon from '../icons/Icon.svelte';
  import { actions, live } from '../live/store.svelte';
  import {
    RELATIVE_TIME_REFRESH_MS,
    SHEET_CLOSE_AFTER_ONLINE_MS,
    UNDO_MS,
  } from '../live/timing';
  import type { ActionHandle, Need } from '../types';
  import type { NameCheck, Outcome } from '../types.gen';
  import Button from '../ui/Button.svelte';
  import Field from '../ui/Field.svelte';
  import Input from '../ui/Input.svelte';
  import { toast } from '../ui/Toast.svelte';
  import AskAdminSheet from './AskAdminSheet.svelte';
  import CheckCode from './CheckCode.svelte';
  import { explain, nameProblem } from './errors';

  let {
    need,
    primary = true,
    onDone,
  }: { need: Need; primary?: boolean; onDone?: () => void } = $props();

  const titleId = $props.id();

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
  let denying = $state(false);
  let failure = $state<string | null>(null);

  const settled = $derived(approval ?? denial);
  const conflict = $derived(settled?.state === 'conflict' ? settled.conflict : null);

  function approve(): void {
    if (!join || frozen || approval || denying || expired) return;
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
    denyTimer = setTimeout(sendDeny, UNDO_MS);
  }

  function undoDeny(): void {
    if (denyTimer !== null) clearTimeout(denyTimer);
    denyTimer = null;
    denying = false;
    needs.release(need.id);
  }

  onDestroy(() => {
    // Leaving the page does not cancel a deny: it is sent now instead of after Undo.
    if (denyTimer !== null) {
      clearTimeout(denyTimer);
      sendDeny();
    }
  });

  const joined = $derived(
    join && approval?.state !== 'failed' && approval?.state !== 'conflict'
      ? (live.peers[join.network]?.[join.node] ?? null)
      : null,
  );
  const joinedOnline = $derived(approval !== null && joined?.online === true);
  const joinedNumbers = $derived(join ? live.numbers[`${join.network}/${join.node}`] : undefined);

  function pathText(): string {
    const path = joinedNumbers?.path ?? joined?.path ?? null;
    const relay = joinedNumbers?.relay ? deviceName(join?.network ?? '', joinedNumbers.relay) : '';
    if (path === 'relay') return copy.path.via(relay);
    if (path === 'stealth') return copy.path.stealthVia(relay);
    return copy.path.direct;
  }

  function progress(handle: ActionHandle<Outcome>): string {
    if (handle.state === 'pending') return copy.action.signing;
    if (handle.state === 'awaiting_os_auth') return copy.action.awaitingOsAuth;
    if (handle.seen) return copy.action.spreading(handle.seen.by, handle.seen.of);
    if (handle.slow) return copy.action.savedWaiting;
    return copy.action.savedHere;
  }

  // Close after the device is online, unless the person is in the card (§7.3 step 5).
  let card = $state<HTMLElement | null>(null);
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
      approve();
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

  function ignore(): void {
    if (!blocked) return;
    const id = blocked.id;
    prefs.ignore(id, live.now());
    toast.done(copy.blocked.ignored, { undo: () => prefs.unignore(id) });
    onDone?.();
  }

  // Health -------------------------------------------------------------------------

  const health = $derived(detail?.kind === 'health' ? detail : null);

  function dismissWarning(): void {
    if (!health) return;
    prefs.ignore(health.id, live.now());
    onDone?.();
  }
</script>

{#if detail}
  <article
    bind:this={card}
    class="card"
    aria-labelledby={titleId}
    {@attach listen}
  >
    {#if join}
      <h3 id={titleId} class="title">{copy.join.wants(join.claimed.name, network)}</h3>
      {#if twin && !settled}
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
        <p class="outcome" role="status">
          {conflict.resolution === 'denied'
            ? copy.join.deniedBy(deviceName(join.network, conflict.resolved_by), relative(conflict.at_ms, now))
            : copy.join.lostRace(deviceName(join.network, conflict.resolved_by), relative(conflict.at_ms, now))}
        </p>
        <div class="actions"><Button variant="ghost" onclick={finish}>{copy.action.dismiss}</Button></div>
      {:else if expired && !approval}
        <p class="outcome" role="status">{copy.join.expired(join.claimed.name)}</p>
        <div class="actions"><Button variant="ghost" onclick={finish}>{copy.action.dismiss}</Button></div>
      {:else if approval}
        <p class="outcome" role="status">
          {#if joinedOnline && joined}
            {copy.join.online(joined.name, pathText(), rtt(joinedNumbers?.rttMs ?? null))}
          {:else}
            {progress(approval)}
          {/if}
        </p>
        {#if joinedOnline}
          <div class="actions"><Button onclick={showDevice}>{copy.join.showDevice}</Button></div>
        {/if}
      {:else if denying}
        <p class="outcome" role="status">
          <span>{copy.join.denied(join.claimed.name)}</span>
          {#if !denial}
            <Button variant="ghost" onclick={undoDeny}>{copy.action.undo}</Button>
          {/if}
        </p>
      {:else}
        <p class="hint">{copy.join.approveOnlyIf}</p>
        <Field label={copy.join.name} error={nameError}>
          {#snippet children(control)}
            <Input
              {...control}
              kind="name"
              bind:value={name}
              onblur={() => void validate()}
              oninput={() => nameError !== null && void validate()}
              onkeydown={(e) => e.key === 'Enter' && approve()}
            />
          {/snippet}
        </Field>
        {#if suggestion}
          <div><Button variant="ghost" onclick={useSuggestion}>{copy.rename.use(suggestion)}</Button></div>
        {/if}
        {#if failure}<p class="error" role="alert">{failure}</p>{/if}
        <div class="actions">
          <Button disabled={frozen} onclick={deny}>{copy.join.deny}</Button>
          <Button variant={primary ? 'primary' : 'secondary'} disabled={frozen} onclick={approve}>
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
        {#if admin}
          <Button disabled={frozen} onclick={() => (allowOpen = true)}>{copy.blocked.allowThis}</Button>
        {:else}
          <Button onclick={() => (asking = true)}>{copy.exits.askAdmin}</Button>
        {/if}
      </div>
      {#if frozen && admin}<p class="reason">{copy.conn.staleActions}</p>{/if}
      {#if admin}
        <AllowBlocked bind:open={allowOpen} id={blocked.id} />
      {:else if asking}
        <AskAdminSheet
          net={blocked.network}
          request={copy.askAdmin.request.allow(
            network,
            blocked.from_name,
            blocked.to_name,
            blocked.protocol,
            blocked.port,
          )}
        />
      {/if}
    {:else if health}
      <p id={titleId} class="line">
        <Icon name={health.fix === null ? 'triangle-alert' : 'circle-x'} />
        <span>{health.what}</span>
      </p>
      {#if health.fix}<p class="hint">{health.fix}</p>{/if}
      {#if health.atMs !== null}
        <div class="actions between">
          <time class="meta" datetime={new Date(health.atMs).toISOString()}>{relative(health.atMs, now)}</time>
          <Button variant="ghost" onclick={dismissWarning}>{copy.action.dismiss}</Button>
        </div>
      {/if}
    {/if}
  </article>
{/if}

<style>
  .card {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-4);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface);
    box-shadow: inset var(--lead-bar) 0 0 var(--attn);
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
  .hint {
    color: var(--ink-2);
    max-width: var(--measure);
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
    justify-content: flex-end;
    align-items: center;
    gap: var(--s-3);
  }
  .actions.between {
    justify-content: space-between;
  }
  .outcome {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: var(--control);
    color: var(--ink-2);
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
