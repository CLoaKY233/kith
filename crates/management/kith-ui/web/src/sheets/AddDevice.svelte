<!--
@component
AddDevice (spec §7.3). Asks `What are you adding?`; a phone hands over to AddPhone
(`?add=phone`). A laptop or server gets two tabs:
  - Approve it live (default): the install and `kith join home@studio-mac` commands, then
    `Waiting for a request…` with a still ring (no spinner loop). Each join request for this
    network swaps in a NeedsYouCard (join), which shows the check code and signs the answer.
    Two requests with one name raise the amber band. Once approved the request reads
    `Saved on this device`, then `dev-box is online · direct · 14 ms` with `Show device`, and
    the sheet closes 2 s later unless the pointer or focus is inside it. A request that lapses
    reads `This request expired. Ask dev-box to try again.`
  - Send an invite link: the invite flow (§7.4), with `Create invite` pinned in the footer.
Props:
  - open: boolean, bindable. The screen maps it to the URL query (Back closes it).
  - net: string, the network as the route names it.
  - kind?: 'laptop' | null. Skips the question, as after CreateNetwork's `Laptop or server`.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { announce } from '../lib/announce';
  import { copy } from '../lib/copy';
  import { rtt } from '../lib/format';
  import NeedsYouCard from '../lib/kith/NeedsYouCard.svelte';
  import { live } from '../lib/live/store.svelte';
  import { SHEET_CLOSE_AFTER_ONLINE_MS } from '../lib/live/timing';
  import type { JoinRequest, PeerInfo } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import CopyField from '../lib/ui/CopyField.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import { go, href, setQuery } from '../router';
  import { deviceName, findNetwork, isStale, membershipOf, pathText } from './flow';
  import { InviteFlow } from './inviteFlow.svelte';
  import InviteBody from './InviteBody.svelte';

  let {
    open = $bindable(false),
    net,
    kind = null,
  }: { open?: boolean; net: string; kind?: 'laptop' | null } = $props();

  type Tab = 'live' | 'invite';

  /** One request this sheet has shown, and where it stands now. */
  type Shown =
    | { readonly state: 'asking'; readonly request: JoinRequest }
    | { readonly state: 'expired'; readonly request: JoinRequest }
    | { readonly state: 'saved'; readonly request: JoinRequest; readonly peer: PeerInfo }
    | { readonly state: 'online'; readonly request: JoinRequest; readonly peer: PeerInfo; readonly line: string }
    | { readonly state: 'gone'; readonly request: JoinRequest };

  let laptop = $state(false);
  let tab = $state<Tab>('live');
  /** Every request for this network seen while the sheet is open, oldest first. */
  let tracked = $state.raw<JoinRequest[]>([]);
  /** Requests already announced as online, so each closes the sheet and speaks once. */
  let settled: string[] = [];
  let body = $state<HTMLElement | null>(null);

  const network = $derived(findNetwork(net));
  const networkId = $derived(network?.id ?? null);
  const me = $derived(networkId === null ? null : membershipOf(networkId));
  const command = $derived(
    network && me ? copy.addDevice.joinCommand(network.name, me.name) : null,
  );
  const flow = new InviteFlow(() => networkId, () => live.now());
  const stale = $derived(isStale());
  /** Server time, moving with each tick, so expiry shows without a timer of its own. */
  const nowMs = $derived(Math.max(live.lastTickMs, live.now()));

  // Each opening starts over: the question (or the laptop tabs), approving live.
  $effect(() => {
    if (!open) return;
    laptop = kind === 'laptop';
    tab = 'live';
    tracked = [];
    settled = [];
    flow.reset();
  });

  // The sheet listens for requests: each new one for this network joins the list.
  $effect(() => {
    if (!open || networkId === null) return;
    const id = networkId;
    const joins = live.pending.joins;
    untrack(() => {
      const fresh = joins.filter((j) => j.network === id && !tracked.some((t) => t.id === j.id));
      if (fresh.length > 0) tracked = [...tracked, ...fresh];
    });
  });

  /** `dev-box is online · direct · 14 ms`, once its path is known. */
  function onlineLine(id: string, peer: PeerInfo): string | null {
    const numbers = live.numbers[`${id}/${peer.node}`];
    const path = pathText(numbers?.path ?? peer.path, deviceName(id, numbers?.relay ?? null));
    return path === null ? null : copy.join.online(peer.name, path, rtt(numbers?.rttMs ?? null));
  }

  const shown = $derived.by((): Shown[] => {
    const id = networkId;
    if (id === null) return [];
    const asking = new Set(live.pending.joins.map((j) => j.id));
    return tracked.map((request): Shown => {
      const peer = live.peers[id]?.[request.node];
      const line = peer?.online ? onlineLine(id, peer) : null;
      if (peer && line !== null) return { state: 'online', request, peer, line };
      if (peer) return { state: 'saved', request, peer };
      if (nowMs >= request.expires_at_ms) return { state: 'expired', request };
      if (asking.has(request.id)) return { state: 'asking', request };
      return { state: 'gone', request };
    });
  });

  const visible = $derived(shown.filter((s) => s.state !== 'gone'));

  /** Names asked for by more than one waiting request (§7.3 step 4). */
  const sharedNames = $derived.by(() => {
    const names = shown.filter((s) => s.state === 'asking').map((s) => s.request.claimed.name);
    return [...new Set(names)].filter((name) => names.indexOf(name) !== names.lastIndexOf(name));
  });

  function inside(): boolean {
    const sheet = body?.closest('.kith-sheet');
    if (!sheet || !body) return false;
    return sheet.matches(':hover') || body.contains(document.activeElement);
  }

  // A device that comes online closes the sheet 2 s later, unless the person is in it.
  $effect(() => {
    for (const s of shown) {
      if (s.state !== 'online' || settled.includes(s.request.id)) continue;
      settled = [...settled, s.request.id];
      announce(s.line);
      setTimeout(() => {
        if (open && !inside()) open = false;
      }, SHEET_CLOSE_AFTER_ONLINE_MS);
    }
  });

  async function chooseLaptop(): Promise<void> {
    laptop = true;
    await tick();
    body?.querySelector<HTMLElement>('[role="tab"][data-state="active"]')?.focus();
  }

  function choosePhone(): void {
    setQuery({ add: 'phone' }, { replace: true });
  }

  function showDevice(peer: PeerInfo): void {
    if (network === null) return;
    open = false;
    go(href({ name: 'device', net: network.name, dev: peer.name }));
  }

  const tabs = [
    { value: 'live', label: copy.addDevice.approveLive },
    { value: 'invite', label: copy.addDevice.sendInvite },
  ] as const;

  const showFooter = $derived(laptop && tab === 'invite' && (flow.invite === null || flow.expired));
</script>

{#snippet createInvite()}
  {#if stale}<p class="reason">{copy.conn.staleActions}</p>{/if}
  <Button
    variant="primary"
    pending={flow.pending}
    disabled={stale}
    onclick={() => (flow.expired ? flow.reset() : flow.create())}
  >
    {copy.invite.create}
  </Button>
{/snippet}

{#snippet approveLive()}
  <div class="live">
    <div class="step">
      <p class="label">{copy.addDevice.notInstalled}</p>
      <CopyField value={copy.addDevice.installCommand} label={copy.addDevice.notInstalled} />
    </div>
    {#if command}
      <div class="step">
        <p class="label">{copy.addDevice.runOnNew}</p>
        <CopyField value={command} label={copy.addDevice.runOnNew} />
      </div>
    {/if}

    {#if visible.length === 0}
      <p class="waiting"><span class="ring" aria-hidden="true"></span>{copy.addDevice.waiting}</p>
    {:else}
      <div class="requests">
        {#each sharedNames as name (name)}
          <p class="band">{copy.join.twoSameName(name)}</p>
        {/each}
        {#each visible as s (s.request.id)}
          {#if s.state === 'asking'}
            <NeedsYouCard
              need={{ id: s.request.id, kind: 'join', network: s.request.network }}
              primary={s.request.id === visible.find((v) => v.state === 'asking')?.request.id}
              twinBand={false}
            />
          {:else if s.state === 'saved'}
            <p class="outcome">{copy.action.savedHere}</p>
          {:else if s.state === 'online'}
            <div class="outcome online">
              <p>{s.line}</p>
              <Button variant="secondary" onclick={() => showDevice(s.peer)}>
                {copy.join.showDevice}
              </Button>
            </div>
          {:else if s.state === 'expired'}
            <p class="outcome quiet">{copy.join.expired(s.request.claimed.name)}</p>
          {/if}
        {/each}
      </div>
    {/if}
  </div>
{/snippet}

<Sheet
  bind:open
  title={copy.addDevice.title}
  closeLabel={copy.action.close}
  footer={showFooter ? createInvite : undefined}
>
  <div class="body" bind:this={body}>
    {#if network === null}
      <!-- The route names no network this device is in: nothing to add to. -->
    {:else if !network.admin}
      <p class="quiet">{copy.error.permissionDenied}</p>
    {:else if !laptop}
      <p class="t-heading">{copy.addDevice.question}</p>
      <div class="kinds">
        <button type="button" class="kind" onclick={chooseLaptop}>
          {copy.addDevice.laptop}
        </button>
        <button type="button" class="kind" onclick={choosePhone}>{copy.addDevice.phone}</button>
      </div>
    {:else}
      <Tabs
        bind:value={tab}
        tabs={tabs}
        label={copy.addDevice.laptop}
        panel={laptopPanel}
      />
    {/if}
  </div>
</Sheet>

{#snippet laptopPanel(value: string)}
  {#if value === 'live'}
    {@render approveLive()}
  {:else if networkId !== null}
    <InviteBody {flow} {networkId} />
  {/if}
{/snippet}

<style>
  .body {
    display: grid;
    gap: var(--s-4);
  }
  .kinds {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--s-3);
  }
  .kind {
    min-height: var(--s-8);
    padding: var(--s-4);
    border-radius: var(--r-3);
    border: var(--hairline) solid var(--line-strong);
    background: var(--surface);
    color: var(--ink-1);
    font-weight: 600;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .kind:hover {
    background: var(--sunken);
  }
  .kind:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .live,
  .requests {
    display: grid;
    gap: var(--s-4);
  }
  .step {
    display: grid;
    gap: var(--s-1);
  }
  .label {
    font-weight: 600;
  }
  .waiting {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-2);
  }
  .ring {
    flex: none;
    width: var(--spinner);
    height: var(--spinner);
    border-radius: var(--r-full);
    border: var(--dot-ring) solid var(--ink-3);
  }
  .band {
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-2);
    background: var(--attn-wash);
    color: var(--attn-text);
  }
  .outcome {
    color: var(--ink-1);
  }
  .online {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .quiet {
    color: var(--ink-3);
  }
  .reason {
    flex: 1;
    align-self: center;
    font: var(--t-small);
    color: var(--ink-3);
  }
  @media (width < 40rem) {
    .kinds {
      grid-template-columns: 1fr;
    }
  }
</style>
