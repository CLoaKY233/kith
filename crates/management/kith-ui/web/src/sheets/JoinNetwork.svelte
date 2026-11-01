<!--
@component
JoinNetwork (spec §7.2): this device joins a network.
  - With a link: one input, `Invite link`. A pasted `kith join …` command works too, and a
    `home@studio-mac` or `kith://request/` code switches to asking. A recognised link is
    previewed: `Join home, invited by studio-mac. Link valid for 47 more minutes.` and the
    button reads `Join home`. An expired or used invite says so.
  - Without one: `I don't have a link — ask to join` asks an admin, given the network and
    admin as the admin's Add device screen shows them. The sheet shows `Your request: kith
    join home@studio-mac`, the check code (`the admin will see the same code`) and `Waiting
    for an admin of home…`, asks again on every tick, and ends with `You're in. 6 devices
    online.` (then enters the network), `An admin denied this request.` or a lapse.
  - While joining or the first ask cannot reach the network: `Can't reach any device in home yet.
    Trying… (12 s)`, and after 60 s `Still no answer. Is one of home's devices online?`
Props:
  - open: boolean, bindable. The screen maps it to the URL query (Back closes it).
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { announce } from '../lib/announce';
  import { copy } from '../lib/copy';
  import CheckCode from '../lib/kith/CheckCode.svelte';
  import { actions, live } from '../lib/live/store.svelte';
  import {
    JOIN_RETRY_HINT_MS,
    RELATIVE_TIME_REFRESH_MS,
    SHEET_CLOSE_AFTER_ONLINE_MS,
  } from '../lib/live/timing';
  import { relative } from '../lib/format';
  import type { ActionHandle, ApiFailure, InvitePreview } from '../lib/types';
  import type { JoinRequestState, Outcome } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import { host as thisHost } from '../frame/host.svelte';
  import { go, href } from '../router';
  import { failureText, isStale, recognise, type JoinTarget } from './flow';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  type Mode = 'link' | 'ask';

  let mode = $state<Mode>('link');
  let linkText = $state('');
  let askText = $state('');
  let fieldError = $state<string | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let requestBlock = $state<HTMLElement | null>(null);

  let joining = $state<ActionHandle<Outcome> | null>(null);
  /** The request being asked, and the latest answer about it. */
  let request = $state<{ code: string; network: string | null; admin: string | null } | null>(null);
  let asking = $state<ActionHandle<JoinRequestState> | null>(null);
  let answer = $state<JoinRequestState | null>(null);
  /** What the link in the field says, once the server decoded it. */
  let preview = $state<{ link: string; value: InvitePreview } | null>(null);
  let askStartedAt = $state(0);
  let elapsedMs = $state(0);
  /** Set once the daemon let this device in: the network to enter, by id if known. */
  let entering = $state<{ id: string | null; before: readonly string[] } | null>(null);

  const host = $derived(thisHost.name);
  const stale = $derived(isStale());
  const joinPending = $derived(joining?.state === 'pending');
  const firstAskPending = $derived(answer === null && asking?.state === 'pending');
  const failure = $derived.by(() => {
    // A failed re-ask while waiting is retried on the next tick, so only the first one shows.
    const askFailed = asking?.state === 'failed' && answer === null ? asking : null;
    const failed = joining?.state === 'failed' ? joining : askFailed;
    if (!failed?.error) return null;
    return failed === joining ? joinFailureText(failed.error) : failureText(failed.error);
  });
  /** The preview of the link now in the field, if it is the one previewed. */
  const shownPreview = $derived.by(() => {
    if (preview === null || mode !== 'link') return null;
    const target = recognise(linkText);
    return target?.kind === 'invite' && target.link === preview.link ? preview.value : null;
  });
  const parsed = $derived.by(() => {
    if (shownPreview === null) return null;
    const minutes = Math.ceil((shownPreview.expires_at_ms - live.now()) / 60_000);
    if (minutes <= 0) return null;
    return copy.joinNetwork.parsed(shownPreview.network_name, shownPreview.invited_by, minutes);
  });
  /** The network being reached, while the link join or the first ask waits for it. */
  const reaching = $derived(
    firstAskPending
      ? (answer?.network_name ?? request?.network ?? null)
      : joinPending
        ? (preview?.value.network_name ?? null)
        : null,
  );
  const resolution = $derived(answer?.resolution ?? null);

  $effect(() => {
    if (failure) announce(failure);
  });
  $effect(() => {
    if (resolution === 'denied') announce(copy.joinNetwork.denied);
    else if (resolution === 'expired') announce(copy.joinNetwork.requestExpired);
  });
  const waiting = $derived(answer !== null && resolution === null);
  const command = $derived(
    request?.network && request.admin
      ? copy.joinNetwork.requestCommand(request.network, request.admin)
      : null,
  );
  /** Whole seconds the first ask has been trying. */
  const trying = $derived(Math.floor(elapsedMs / 1000));

  /** The network this device just entered, once the status lists it. */
  const entered = $derived.by(() => {
    if (entering === null) return null;
    const { id, before } = entering;
    const networks = live.status?.networks ?? [];
    return (
      networks.find((n) => id !== null && n.id === id) ??
      networks.find((n) => !before.includes(n.id)) ??
      null
    );
  });

  // Each opening starts over with an empty link field, focused.
  $effect(() => {
    if (!open) return;
    mode = 'link';
    linkText = '';
    askText = '';
    fieldError = null;
    joining = null;
    request = null;
    asking = null;
    answer = null;
    entering = null;
    enteredName = null;
    preview = null;
    void tick().then(() => input?.focus());
  });

  // The seconds count while the join or the first ask is still trying to reach the network.
  $effect(() => {
    if (!firstAskPending && !joinPending) return;
    const timer = setInterval(
      () => (elapsedMs = Date.now() - askStartedAt),
      RELATIVE_TIME_REFRESH_MS,
    );
    return () => clearInterval(timer);
  });

  // While an admin decides, ask again once per tick (the daemon answers with the decision).
  let polledAt = 0;
  $effect(() => {
    const at = live.lastTickMs;
    if (!waiting || request === null || stale || at === polledAt) return;
    if (asking?.state === 'pending') return;
    polledAt = at;
    void ask(request.code);
  });

  // Approved: the page enters the network as soon as the status lists it.
  $effect(() => {
    if (resolution === 'approved' && entering === null) {
      entering = { id: answer?.network ?? null, before: networkIds() };
    }
  });

  let enteredName: string | null = null;
  $effect(() => {
    const network = entered;
    if (network === null || network.name === enteredName) return;
    enteredName = network.name;
    announce(copy.joinNetwork.joined(network.peers_online));
    setTimeout(() => {
      open = false;
      go(href({ name: 'overview', net: network.name }));
    }, SHEET_CLOSE_AFTER_ONLINE_MS);
  });

  /** Keeps focus on the outcome when the request it replaced goes away. */
  function focusOnMount(node: HTMLElement): void {
    node.focus();
  }

  /** A failed join, in words: an expired or used invite says so (§7.2 step 3). */
  function joinFailureText(error: ApiFailure): string {
    const code: string = error.code;
    const words = error.message.toLowerCase();
    if (code === 'invite_used' || /\b(already )?used\b/.test(words)) return copy.joinNetwork.used;
    if (code === 'invite_expired' || words.includes('expired')) {
      const known = preview?.value;
      return known
        ? copy.joinNetwork.expired(relative(known.expires_at_ms, live.now()), known.invited_by)
        : copy.joinNetwork.expiredPlain;
    }
    return failureText(error);
  }

  /** Asks the server what a recognised link says, for the line under the field. */
  async function previewLink(link: string): Promise<void> {
    if (preview?.link === link) return;
    const settled = await actions.previewInvite(link).done;
    if (settled.result !== null) preview = { link, value: settled.result };
  }

  function networkIds(): string[] {
    return (live.status?.networks ?? []).map((n) => n.id);
  }

  async function ask(code: string): Promise<void> {
    const handle = actions.requestJoin(code, host);
    asking = handle;
    const settled = await handle.done;
    if (settled.result === null || asking !== handle) return;
    const first = answer === null;
    answer = settled.result;
    // The field goes away with the first answer: keep focus on what replaced it.
    if (first) {
      await tick();
      requestBlock?.focus();
    }
  }

  async function startAsking(target: Extract<JoinTarget, { kind: 'request' }>): Promise<void> {
    request = { code: target.code, network: target.network, admin: target.admin };
    answer = null;
    askStartedAt = Date.now();
    elapsedMs = 0;
    await ask(target.code);
  }

  async function join(link: string): Promise<void> {
    const before = networkIds();
    askStartedAt = Date.now();
    elapsedMs = 0;
    const handle = actions.join(link, host);
    joining = handle;
    const settled = await handle.done;
    if (settled.state === 'accepted' || settled.state === 'seen') entering = { id: null, before };
  }

  function submit(event?: SubmitEvent): void {
    event?.preventDefault();
    if (stale || joinPending || asking?.state === 'pending') return;
    joining = null;
    asking = null;
    const text = mode === 'link' ? linkText : askText;
    const target = recognise(text);
    if (target === null) {
      fieldError = mode === 'link' ? copy.joinNetwork.notALink : copy.joinNetwork.notARequest;
      input?.focus();
      return;
    }
    fieldError = null;
    if (target.kind === 'invite') {
      mode = 'link';
      linkText = target.link;
      void join(target.link);
    } else {
      mode = 'ask';
      askText = target.code;
      void startAsking(target);
    }
  }

  async function switchTo(next: Mode): Promise<void> {
    mode = next;
    fieldError = null;
    joining = null;
    asking = null;
    await tick();
    input?.focus();
  }

  function revalidate(): void {
    const target = recognise(mode === 'link' ? linkText : askText);
    if (target?.kind === 'invite') void previewLink(target.link);
    if (fieldError !== null && target !== null) fieldError = null;
  }

  /** The field stays until an answer arrives, and comes back to ask again. */
  const showForm = $derived(answer === null || resolution === 'denied' || resolution === 'expired');
</script>

{#snippet primary()}
  {#if stale}<p class="reason">{copy.conn.staleActions}</p>{/if}
  <Button
    variant="primary"
    type="submit"
    form="join-network"
    pending={joinPending || firstAskPending || entering !== null}
    pendingLabel={mode === 'link' ? copy.joinNetwork.joining : copy.joinNetwork.asking}
    disabled={stale}
  >
    {#if mode === 'ask'}
      {copy.joinNetwork.ask}
    {:else if shownPreview}
      {copy.joinNetwork.submit(shownPreview.network_name)}
    {:else}
      {copy.joinNetwork.title}
    {/if}
  </Button>
{/snippet}

<Sheet
  bind:open
  title={copy.joinNetwork.title}
  closeLabel={copy.action.close}
  footer={entered === null && showForm ? primary : undefined}
>
  {#if entered}
    <p class="t-heading joined" tabindex="-1" use:focusOnMount>
      {copy.joinNetwork.joined(entered.peers_online)}
    </p>
  {:else}
    {#if showForm}
      <form id="join-network" class="form" onsubmit={submit} novalidate>
        {#if mode === 'link'}
          <Field
            label={copy.joinNetwork.label}
            helper={parsed ?? copy.joinNetwork.helper}
            error={fieldError}
          >
            {#snippet children(control)}
              <Input
                {...control}
                bind:ref={input}
                bind:value={linkText}
                kind="mono"
                autocapitalize="none"
                autocorrect="off"
                spellcheck={false}
                enterkeyhint="go"
                oninput={revalidate}
              />
            {/snippet}
          </Field>
        {:else}
          <Field label={copy.joinNetwork.askLabel} helper={copy.joinNetwork.askHelper} error={fieldError}>
            {#snippet children(control)}
              <Input
                {...control}
                bind:ref={input}
                bind:value={askText}
                kind="mono"
                autocapitalize="none"
                autocorrect="off"
                spellcheck={false}
                enterkeyhint="go"
                oninput={revalidate}
              />
            {/snippet}
          </Field>
        {/if}
        <div class="alt">
          <Button variant="ghost" onclick={() => switchTo(mode === 'link' ? 'ask' : 'link')}>
            {mode === 'link' ? copy.joinNetwork.noLink : copy.joinNetwork.haveLink}
          </Button>
        </div>
      </form>
    {/if}

    {#if reaching && trying > 0}
      <div class="trying">
        <p>{copy.joinNetwork.trying(reaching, trying)}</p>
        {#if elapsedMs >= JOIN_RETRY_HINT_MS}
          <p class="quiet">{copy.joinNetwork.stillNothing(reaching)}</p>
        {/if}
      </div>
    {/if}

    {#if answer && resolution !== 'expired'}
      <div class="request" bind:this={requestBlock} tabindex="-1">
        {#if command}
          <p>{copy.joinNetwork.requestLabel} <code class="t-mono">{command}</code></p>
        {/if}
        <CheckCode code={answer.check_code} caption={copy.checkCode.flowLabel} />
        {#if resolution === null}
          <p class="quiet">{copy.joinNetwork.waiting(answer.network_name)}</p>
        {:else if resolution === 'denied'}
          <p class="denied">{copy.joinNetwork.denied}</p>
        {/if}
      </div>
    {:else if resolution === 'expired'}
      <p class="quiet">{copy.joinNetwork.requestExpired}</p>
    {/if}

    {#if failure && !stale}<p class="failed">{failure}</p>{/if}
  {/if}
</Sheet>

<style>
  .form,
  .request,
  .trying {
    display: grid;
    gap: var(--s-3);
    max-width: var(--measure);
  }
  .form {
    gap: var(--s-2);
  }
  /* The quiet alternative lines up with the field, not with its own padding and border. */
  .alt {
    margin-inline-start: calc(-1 * (var(--s-3) + var(--hairline)));
  }
  .alt :global(.btn) {
    color: var(--ink-2);
    font-weight: 400;
  }
  .request:focus,
  .joined:focus {
    outline: none;
  }
  .quiet {
    color: var(--ink-3);
  }
  .denied {
    color: var(--ink-1);
  }
  .failed {
    font: var(--t-small);
    color: var(--danger);
  }
  .reason {
    flex: 1;
    align-self: center;
    font: var(--t-small);
    color: var(--ink-3);
  }
</style>
