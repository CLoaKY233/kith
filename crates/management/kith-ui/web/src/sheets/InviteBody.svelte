<!--
@component
The body of the invite flow (spec §7.4), shared by the Invite sheet and AddDevice. Before the
invite exists: `Valid for` (1 hour · 1 day · 1 week) and `Uses` (1 · 5 · Unlimited). After:
the link in a CopyField, the QR behind `Show QR code`, `Or run: kith join <link>`, the live
status line and `Revoke invite`. The parent renders `Create invite` (calling `flow.create()`)
in its footer, so the primary action stays pinned at the bottom.
Props:
  - flow: InviteFlow, the state this body shows and changes.
  - networkId: string, the network's id.
-->
<script lang="ts">
  import { announce } from '../lib/announce';
  import { copy } from '../lib/copy';
  import { until } from '../lib/format';
  import QrPanel from '../lib/kith/QrPanel.svelte';
  import { actions, live } from '../lib/live/store.svelte';
  import Button from '../lib/ui/Button.svelte';
  import CopyField from '../lib/ui/CopyField.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import { deviceName, failureText, isStale, pathText } from './flow';
  import type { InviteFlow, UsesChoice, Validity } from './inviteFlow.svelte';

  let { flow, networkId }: { flow: InviteFlow; networkId: string } = $props();

  const validityOptions = [
    { value: 'hour', label: copy.invite.validity.hour },
    { value: 'day', label: copy.invite.validity.day },
    { value: 'week', label: copy.invite.validity.week },
  ] as const;
  const usesOptions = [
    { value: 'one', label: copy.invite.useCounts.one },
    { value: 'five', label: copy.invite.useCounts.five },
    { value: 'unlimited', label: copy.invite.useCounts.unlimited },
  ] as const;

  const stale = $derived(isStale());
  const createFailed = $derived(
    flow.creating?.state === 'failed' && flow.creating.error ? failureText(flow.creating.error) : null,
  );
  const revokeFailed = $derived(
    flow.revoking?.state === 'failed' && flow.revoking.error ? failureText(flow.revoking.error) : null,
  );

  $effect(() => {
    const text = createFailed ?? revokeFailed;
    if (text) announce(text);
  });

  /** The latest copy of this invite the server listed, for who has used it. */
  const current = $derived.by(() => {
    const invite = flow.invite;
    if (invite === null) return null;
    return live.invites[networkId]?.find((i) => i.link === invite.link) ?? invite;
  });

  /** Every use taken: the QR has nothing left to offer, so it collapses for good. */
  const spent = $derived(
    current !== null && current.uses !== null && current.used_by.length >= current.uses,
  );

  const status = $derived.by(() => {
    if (current === null) return null;
    const user = current.used_by.at(-1);
    const peer =
      user === undefined
        ? undefined
        : Object.values(live.peers[networkId] ?? {}).find((p) => p.name === user);
    if (peer?.online) {
      const numbers = live.numbers[`${networkId}/${peer.node}`];
      const path = pathText(peer.path, deviceName(networkId, numbers?.relay ?? null));
      if (path !== null) return copy.invite.usedBy(peer.name, path);
    }
    return copy.invite.notUsed(until(current.expires_at_ms, live.now()));
  });

  // A device that joins after the invite was made may have used it: ask the server again.
  let lastSeq = 0;
  $effect(() => {
    const journal = live.journal;
    const since = flow.createdAtMs;
    if (flow.invite === null) return;
    let joined = false;
    for (const entry of journal) {
      if (entry.seq <= lastSeq) continue;
      if (entry.kind === 'device_joined' && entry.network === networkId && entry.at_ms >= since) {
        joined = true;
      }
    }
    lastSeq = journal.at(-1)?.seq ?? lastSeq;
    if (joined) actions.loadInvites(networkId);
  });

  async function revoke(): Promise<void> {
    if (stale) return;
    if (await flow.revoke()) toast.done(copy.invite.revoked);
  }
</script>

{#if flow.invite === null}
  <div class="choices">
    <div class="choice">
      <p class="label" aria-hidden="true">{copy.invite.validFor}</p>
      <Segmented
        label={copy.invite.validFor}
        options={validityOptions}
        value={flow.validity}
        onValueChange={(v) => (flow.validity = v as Validity)}
      />
    </div>
    <div class="choice">
      <p class="label" aria-hidden="true">{copy.invite.uses}</p>
      <Segmented
        label={copy.invite.uses}
        options={usesOptions}
        value={flow.uses}
        onValueChange={(v) => (flow.uses = v as UsesChoice)}
      />
    </div>
    {#if createFailed}<p class="failed">{createFailed}</p>{/if}
  </div>
{:else}
  <div class="result">
    <CopyField value={flow.invite.link} label={copy.invite.link} />
    <QrPanel svg={flow.invite.qr_svg} collapse={spent} />
    <p class="run">{copy.invite.orRun(flow.invite.link)}</p>
    {#if status}<p class="status">{status}</p>{/if}
    <div class="revoke">
      <Button
        variant="danger-quiet"
        pending={flow.revoking?.state === 'pending'}
        disabled={stale}
        onclick={revoke}
      >
        {copy.invite.revoke}
      </Button>
      {#if stale}
        <p class="reason">{copy.conn.staleActions}</p>
      {:else if revokeFailed}
        <p class="failed">{revokeFailed}</p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .choices,
  .result {
    display: grid;
    gap: var(--s-5);
  }
  .choice {
    display: grid;
    justify-items: start;
    gap: var(--s-1);
  }
  .label {
    font-weight: 600;
  }
  .run {
    font: var(--t-small);
    color: var(--ink-2);
    overflow-wrap: anywhere;
  }
  .status {
    color: var(--ink-1);
  }
  .revoke {
    display: grid;
    justify-items: start;
    gap: var(--s-2);
  }
  .reason {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .failed {
    font: var(--t-small);
    color: var(--danger);
  }
</style>
