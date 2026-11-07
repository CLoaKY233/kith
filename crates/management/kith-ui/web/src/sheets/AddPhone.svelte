<!--
@component
AddPhone (spec §7.6): one scrolling page with three numbered steps, no wizard.
  1. Name it: name and tags, then `Create phone config`, the flow's only ledger write
     (tier 1: the fields are the in-place preview of what is signed).
  2. Install WireGuard.
  3. Scan: `Just my devices | All traffic`, then the config. Revealing it is tier 3: the
     server waits for OS confirmation and the page says so. Under a coarse pointer or below
     640 px, `Download .conf` is the primary action and the QR the secondary one.
The status line under the QR follows the phone until its first handshake, then the QR
collapses. The config, which holds the phone's private key, leaves the page when the QR
hides (first handshake, or QR_AUTO_HIDE_MS) and when the mode changes.
Props:
  - open?: boolean, bindable. The screen maps it to `?add=phone` (Back closes it).
  - net: string, the network's id or name.
  - node?: string. A phone made earlier that never connected: the sheet opens at step 3
    (the waiting row's `Show QR code again`).
  - onOpenChange?: (open: boolean) => void.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { MediaQuery } from 'svelte/reactivity';
  import { announce } from '../lib/announce';
  import { copy } from '../lib/copy';
  import { rtt } from '../lib/format';
  import QrPanel from '../lib/kith/QrPanel.svelte';
  import { actions, live } from '../lib/live/store.svelte';
  import {
    IDLE_BYTES_PER_SEC,
    PHONE_NO_INTERNET_MS,
    PHONE_NOT_CONNECTED_MS,
    QR_AUTO_HIDE_MS,
    SPACES_HINT_MS,
  } from '../lib/live/timing';
  import type { ActionHandle } from '../lib/types';
  import type { GatewayInfo, PhoneConfig, PhoneCreated } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import { go, href } from '../router';
  import {
    cannotAct,
    failureText,
    freeName,
    nameProblemText,
    networkContext,
    normalizeInput,
    progressText,
  } from './act';

  type Mode = 'mine' | 'all';
  type Use = 'qr' | 'download' | 'text';

  let {
    open = $bindable(false),
    net,
    node,
    onOpenChange,
  }: {
    open?: boolean;
    net: string;
    node?: string;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const narrow = new MediaQuery('(width < 40rem)');
  const coarse = new MediaQuery('(pointer: coarse)');
  const downloadFirst = $derived(narrow.current || coarse.current);

  const context = $derived(networkContext(live.status, net));
  const netId = $derived(context?.id ?? net);
  const networkName = $derived(context?.name ?? net);
  const peers = $derived(live.peers[netId] ?? {});
  const stale = $derived(cannotAct(live.conn));

  const exits = $derived(live.exits[netId] ?? []);
  /** Only an exit this phone may use; without one, All traffic is unavailable. */
  const exitName = $derived(exits.find((e) => e.allowed)?.name ?? null);

  /** Where phones connect; the server's default, an exit when there is one, comes first. */
  let gateways = $state.raw<GatewayInfo[]>([]);
  let gatewayNode = $state<string | null>(null);
  const chosen = $derived(gateways.find((g) => g.node === gatewayNode) ?? null);
  /** The exit All traffic goes through: the gateway itself when it is one. */
  const allVia = $derived(chosen?.exit ? chosen.name : exitName);

  // Step 1.
  let name = $state('');
  let tags = $state('');
  let nameError = $state<string | null>(null);
  let suggestion = $state<string | null>(null);
  let spacesHint = $state(false);
  let submitting = $state(false);
  let nameInput = $state<HTMLInputElement | null>(null);
  let creating = $state.raw<ActionHandle<PhoneCreated> | null>(null);
  let made = $state.raw<PhoneCreated | null>(null);
  const formId = $props.id();

  // Step 3.
  let mode = $state<Mode>('mine');
  let revealing = $state.raw<ActionHandle<PhoneConfig> | null>(null);
  let config = $state.raw<PhoneConfig | null>(null);
  let qrShown = $state(false);
  let textOpen = $state(false);
  let revealedAt = $state<number | null>(null);
  let qrAt = $state<number | null>(null);
  let connectedAt = $state<number | null>(null);
  let slowToConnect = $state(false);
  let internetDue = $state(false);
  let sawTraffic = $state(false);
  let revealButton = $state<HTMLButtonElement | null>(null);
  let downloadButton = $state<HTMLButtonElement | null>(null);

  /** The phone this sheet is about: made here, or the `node` it was opened for. */
  const phone = $derived.by((): PhoneCreated | null => {
    if (made) return made;
    const waiting = node ? peers[node] : undefined;
    if (!waiting) return null;
    return {
      name: waiting.name,
      node: waiting.node,
      gateway: waiting.gateway ?? '',
      everything_available: allVia !== null,
    };
  });
  const phonePeer = $derived(phone ? (peers[phone.node] ?? null) : null);
  const gateway = $derived(phonePeer?.gateway ?? phone?.gateway ?? '');
  const exitAvailable = $derived(phone ? phone.everything_available : allVia !== null);
  const numbers = $derived(phone ? live.numbers[`${netId}/${phone.node}`] : undefined);

  $effect(() => {
    if (!open) return;
    untrack(() => {
      const taken = Object.values(peers).map((p) => p.name);
      if (context?.membership) taken.push(context.membership.name);
      name = freeName(copy.addPhone.initial, taken);
      tags = copy.addPhone.tagDefault;
      nameError = null;
      suggestion = null;
      creating = null;
      made = null;
      mode = 'mine';
      forget();
      revealing = null;
      revealedAt = null;
      connectedAt = null;
      slowToConnect = false;
      internetDue = false;
      sawTraffic = false;
      if (!live.exits[netId]) actions.loadExits(netId);
      gateways = [];
      gatewayNode = null;
      void loadGateways(netId);
    });
  });

  let last: boolean | null = null;
  $effect(() => {
    const now = open;
    if (last !== null && now !== last) untrack(() => onOpenChange?.(now));
    last = now;
  });

  /** Picks the default gateway, and All traffic when that gateway is an exit. */
  async function loadGateways(id: string): Promise<void> {
    try {
      const list = await actions.phoneGateways(id);
      gateways = list;
      const pick = list.find((g) => g.default) ?? list[0] ?? null;
      gatewayNode = pick?.node ?? null;
      if (pick?.exit) mode = 'all';
    } catch {
      gateways = [];
    }
  }

  /** Drops the private config from the page. */
  function forget(): void {
    config = null;
    qrShown = false;
    qrAt = null;
    textOpen = false;
  }

  let checkSeq = 0;

  /** Checks the name with the CLI's validator; only the latest answer counts. */
  async function validate(): Promise<boolean> {
    const seq = ++checkSeq;
    const settled = await actions.validateName(netId, name).done;
    if (seq !== checkSeq) return false;
    const check = settled.result;
    if (!check) return true;
    nameError = nameProblemText(check, networkName);
    suggestion = check.problem === 'taken' ? check.suggestion : null;
    return nameError === null;
  }

  let hintTimer: ReturnType<typeof setTimeout> | null = null;

  function typed(event: Event & { currentTarget: HTMLInputElement }): void {
    const next = normalizeInput(event.currentTarget);
    name = next.value;
    if (next.spaced) {
      spacesHint = true;
      if (hintTimer !== null) clearTimeout(hintTimer);
      hintTimer = setTimeout(() => (spacesHint = false), SPACES_HINT_MS);
    }
    if (nameError !== null) void validate();
  }

  function useSuggestion(next: string): void {
    name = next;
    void validate();
    nameInput?.focus();
  }

  async function create(): Promise<void> {
    if (stale || submitting) return;
    submitting = true;
    try {
      if (!(await validate())) {
        nameInput?.focus();
        return;
      }
      const list = tags.split(/[\s,]+/).filter(Boolean);
      const handle = actions.createPhone(netId, name, list, gatewayNode ?? undefined);
      creating = handle;
      const settled = await handle.done;
      if (creating === handle && settled.result) {
        made = settled.result;
        // The form and its button leave the page: focus moves to step 3's action.
        await tick();
        (downloadFirst ? downloadButton : revealButton)?.focus();
      }
    } finally {
      submitting = false;
    }
  }

  const createPending = $derived(submitting || creating?.state === 'pending');
  const createProgress = $derived(creating ? progressText(creating) : null);
  const madeTags = $derived(
    tags
      .split(/[\s,]+/)
      .filter(Boolean)
      .join(', '),
  );

  const createFailure = $derived(
    creating?.state === 'failed' && creating.error ? failureText(creating.error, name) : null,
  );

  function changeMode(next: string): void {
    if (next !== 'mine' && next !== 'all') return;
    mode = next;
    forget();
  }

  /** A tier-3 handle waits in `awaiting_os_auth`, not `pending` (§13). */
  const revealPending = $derived(
    revealing?.state === 'pending' || revealing?.state === 'awaiting_os_auth',
  );

  /** Tier 3: asks the server, which waits for OS confirmation, then uses the config. */
  async function reveal(use: Use): Promise<void> {
    if (!phone || stale || revealPending) return;
    if (config) {
      apply(use, config);
      return;
    }
    const handle = actions.phoneConfig(netId, phone.node, mode === 'all');
    revealing = handle;
    const settled = await handle.done;
    if (revealing !== handle) return;
    if (settled.result) apply(use, settled.result);
    else if (use === 'text') textOpen = false;
  }

  function apply(use: Use, next: PhoneConfig): void {
    config = next;
    revealedAt ??= Date.now();
    if (use === 'qr') {
      qrShown = next.qr_svg !== null;
      qrAt = Date.now();
      if (next.qr_svg === null) textOpen = true;
    } else if (use === 'text') {
      textOpen = true;
    } else {
      download(next);
      if (!qrShown && !textOpen) config = null;
    }
  }

  function download(next: PhoneConfig): void {
    if (!phone) return;
    const url = URL.createObjectURL(new Blob([next.config], { type: 'text/plain' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = copy.addPhone.fileName(phone.name);
    link.click();
    setTimeout(() => URL.revokeObjectURL(url));
  }

  function toggleText(event: Event & { currentTarget: HTMLDetailsElement }): void {
    const opened = event.currentTarget.open;
    textOpen = opened;
    if (opened && !config) void reveal('text');
    if (!opened && !qrShown) config = null;
  }

  const revealFailure = $derived(
    revealing?.state === 'failed' && revealing.error && phone
      ? failureText(revealing.error, phone.name)
      : null,
  );

  // The config leaves the page once neither the QR nor the text shows it.
  $effect(() => {
    if (config && !qrShown && !textOpen) untrack(forget);
  });

  // First handshake: the QR collapses and the config leaves the page.
  $effect(() => {
    if (!phonePeer?.online || connectedAt !== null) return;
    untrack(() => {
      connectedAt = Date.now();
      forget();
    });
  });

  $effect(() => {
    if (revealedAt === null || connectedAt !== null) return;
    const timer = setTimeout(
      () => (slowToConnect = true),
      Math.max(0, revealedAt + PHONE_NOT_CONNECTED_MS - Date.now()),
    );
    return () => clearTimeout(timer);
  });

  $effect(() => {
    if (qrAt === null) return;
    const timer = setTimeout(forget, Math.max(0, qrAt + QR_AUTO_HIDE_MS - Date.now()));
    return () => clearTimeout(timer);
  });

  $effect(() => {
    if (connectedAt === null) return;
    const timer = setTimeout(
      () => (internetDue = true),
      Math.max(0, connectedAt + PHONE_NO_INTERNET_MS - Date.now()),
    );
    return () => clearTimeout(timer);
  });

  $effect(() => {
    if (connectedAt !== null && (numbers?.rx ?? 0) >= IDLE_BYTES_PER_SEC) sawTraffic = true;
  });

  const connected = $derived(connectedAt !== null && phone !== null);
  const connectedLine = $derived(
    phone ? copy.addPhone.connected(phone.name, gateway, rtt(numbers?.rttMs ?? null)) : '',
  );

  // Results of the person's own actions reach screen readers through the announcer (§11.1).
  $effect(() => {
    if (connected) untrack(() => announce(connectedLine));
  });
  $effect(() => {
    if (createFailure) announce(createFailure);
  });
  $effect(() => {
    if (revealFailure) announce(revealFailure);
  });

  const status = $derived.by(() => {
    if (connected) return connectedLine;
    if (revealedAt === null) return null;
    return slowToConnect ? copy.addPhone.notYet(gateway) : copy.addPhone.waiting(phone?.name ?? '');
  });

  const exitsHref = $derived(href({ name: 'exits', net: networkName }));

  /**
   * Keeps the phone in this entry's URL first, so Back from Exits reopens step 3 for it
   * (there is no other way back to an unconnected phone's config yet).
   */
  function toExits(event: MouseEvent): void {
    event.preventDefault();
    if (phone) {
      const here = new URL(location.href);
      here.searchParams.set('node', phone.node);
      history.replaceState(history.state, '', `${here.pathname}${here.search}`);
    }
    go(exitsHref);
  }

  function showDevice(): void {
    if (!phone) return;
    open = false;
    go(href({ name: 'device', net: networkName, dev: phone.name }));
  }
</script>

<Sheet bind:open title={copy.addPhone.title} closeLabel={copy.action.close}>
  <ol class="steps">
    <li class="step">
      <h3 class="t-heading">{copy.addPhone.stepName}</h3>
      {#if phone}
        {#if made}<p class="fact">{copy.addPhone.made(made.name, madeTags)}</p>{/if}
        <p class="fact">{copy.addPhone.helper(phone.name, networkName)}</p>
        {#if made && createProgress}<p class="note">{createProgress}</p>{/if}
      {:else}
        <form
          id={formId}
          class="fields"
          onsubmit={(event) => {
            event.preventDefault();
            void create();
          }}
        >
          <Field
            label={copy.addPhone.name}
            helper={name && !nameError ? copy.addPhone.helper(name, networkName) : undefined}
            error={nameError}
            hint={spacesHint ? copy.createNetwork.spaces : null}
          >
            {#snippet children(control)}
              <Input
                {...control}
                kind="name"
                bind:value={name}
                bind:ref={nameInput}
                readonly={createPending}
                oninput={typed}
                onblur={() => void validate()}
              />
            {/snippet}
          </Field>
          {#if suggestion}
            <div>
              <Button onclick={() => useSuggestion(suggestion ?? '')}>
                {copy.rename.use(suggestion)}
              </Button>
            </div>
          {/if}
          <Field label={copy.addPhone.tags}>
            {#snippet children(control)}
              <Input {...control} kind="name" bind:value={tags} readonly={createPending} />
            {/snippet}
          </Field>
          {#if createFailure}
            <p class="fail">{createFailure}</p>
          {:else if createProgress}
            <p class="note">{createProgress}</p>
          {/if}
        </form>
      {/if}
    </li>

    <li class="step">
      <h3 class="t-heading">{copy.addPhone.stepInstall}</h3>
      <p class="prose">{copy.addPhone.install}</p>
    </li>

    <li class="step">
      <h3 class="t-heading">{copy.addPhone.stepScan}</h3>
      {#if !phone}
        <p class="note">{copy.addPhone.scanLater}</p>
      {:else if !connected}
        <div class="mode">
          <Segmented
            value={mode}
            label={copy.addPhone.routeLabel}
            options={[
              { value: 'mine', label: copy.addPhone.justMine },
              {
                value: 'all',
                label: allVia ? copy.addPhone.everythingVia(allVia) : copy.addPhone.everything,
                disabled: !exitAvailable,
                reason: copy.addPhone.noExitReason,
              },
            ]}
            onValueChange={changeMode}
          />
          {#if !exitAvailable}
            <a class="link" href={exitsHref} onclick={toExits}>{copy.addPhone.noExitLink}</a>
          {:else}
            <p class="note">
              {mode === 'all'
                ? copy.addPhone.everythingHelp(allVia ?? '')
                : copy.addPhone.justMineHelp(networkName)}
            </p>
          {/if}
        </div>
      {/if}

      {#if qrShown && config}
        <QrPanel svg={config.qr_svg} bind:shown={qrShown} collapse={connected} />
      {/if}

      {#if revealPending}
        <p class="note">{copy.tier3.waiting}</p>
      {:else if revealFailure}
        <p class="fail">{revealFailure}</p>
      {/if}

      {#if status}
        <p class="status">{status}</p>
      {/if}
      {#if connected && mode === 'all' && internetDue && !sawTraffic}
        <p class="note">{copy.addPhone.noInternet}</p>
      {/if}

      {#if phone && !connected}
        <details class="text" open={textOpen} ontoggle={toggleText}>
          <summary>{copy.addPhone.showText}</summary>
          {#if config}<pre class="config">{config.config}</pre>{/if}
        </details>
      {/if}
      {#if stale}<p class="note">{copy.conn.staleActions}</p>{/if}
    </li>
  </ol>

  {#snippet footer()}
    {#if !phone}
      <Button
        variant="primary"
        type="submit"
        form={formId}
        disabled={stale}
        pending={createPending}
      >
        {copy.addPhone.create}
      </Button>
    {:else if connected}
      <Button onclick={() => (open = false)}>{copy.action.close}</Button>
      <Button variant="primary" onclick={showDevice}>{copy.join.showDevice}</Button>
    {:else}
      {#if downloadFirst}
        {#if !qrShown}
          <Button
            bind:ref={revealButton}
            disabled={stale}
            pending={revealPending}
            onclick={() => void reveal('qr')}
          >
            {copy.addPhone.reveal}
          </Button>
        {/if}
        <Button
          bind:ref={downloadButton}
          variant="primary"
          disabled={stale}
          pending={revealPending}
          onclick={() => void reveal('download')}
        >
          {copy.addPhone.download}
        </Button>
      {:else}
        <Button
          bind:ref={downloadButton}
          disabled={stale}
          pending={revealPending}
          onclick={() => void reveal('download')}
        >
          {copy.addPhone.download}
        </Button>
        {#if !qrShown}
          <Button
            bind:ref={revealButton}
            variant="primary"
            disabled={stale}
            pending={revealPending}
            onclick={() => void reveal('qr')}
          >
            {copy.addPhone.reveal}
          </Button>
        {/if}
      {/if}
    {/if}
  {/snippet}
</Sheet>

<style>
  .steps {
    display: grid;
    gap: var(--s-6);
    counter-reset: step;
  }
  .step {
    display: grid;
    gap: var(--s-3);
    counter-increment: step;
  }
  .step > h3::before {
    content: counter(step);
    display: inline-grid;
    place-items: center;
    width: var(--chip);
    height: var(--chip);
    margin-right: var(--s-2);
    border-radius: var(--r-full);
    background: var(--sunken);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    color: var(--ink-2);
    vertical-align: text-bottom;
  }
  .fields {
    display: grid;
    gap: var(--s-4);
  }
  .fact,
  .prose {
    color: var(--ink-2);
    max-width: var(--measure);
  }
  .mode {
    display: grid;
    gap: var(--s-1);
  }
  .note {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .link {
    font: var(--t-small);
    color: var(--ink-1);
    text-decoration: underline;
    text-underline-offset: var(--s-1);
  }
  .link:focus-visible,
  summary:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .status {
    color: var(--ink-1);
  }
  .fail {
    color: var(--danger);
  }
  summary {
    cursor: pointer;
    font-weight: 600;
    border-radius: var(--r-2);
  }
  .config {
    margin-top: var(--s-2);
    padding: var(--s-3);
    border-radius: var(--r-2);
    background: var(--sunken);
    font: var(--t-mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
