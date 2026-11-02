<!--
@component
The body of the invite flow (spec §7.4), shared by the Invite sheet and AddDevice. Before the
invite exists: `Valid for` (1 hour · 1 day · 1 week) and `Uses` (1 · 5 · Unlimited). After:
the link in a CopyField, the QR behind `Show QR code` (hidden again on each use), `Or run:
kith join <link>` as a command, the live status line (a polite live region) and `Revoke
invite` (instant, with Undo). Once expired it says `Expired at 04:27` and offers nothing
dead. The parent renders `Create invite` (calling `flow.create()`) in its footer, so the
primary action stays pinned at the bottom.
Props:
  - flow: InviteFlow, the state this body shows and changes.
  - networkId: string, the network's id.
-->
<script lang="ts">
  import { announce } from '../lib/announce';
  import { copy } from '../lib/copy';
  import { clock } from '../lib/format';
  import QrPanel from '../lib/kith/QrPanel.svelte';
  import { actions, live } from '../lib/live/store.svelte';
  import Button from '../lib/ui/Button.svelte';
  import CopyField from '../lib/ui/CopyField.svelte';
  import Segmented from '../lib/ui/Segmented.svelte';
  import { toast } from '../lib/ui/Toast.svelte';
  import type { Invite } from '../lib/types.gen';
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

  $effect(() => {
    if (createFailed) announce(createFailed);
  });

  /**
   * The invite as made, with who has used it: the devices that joined this network since
   * it was made (an invite's uses are the joins), and the uses the server says are left.
   */
  const current = $derived.by(() => {
    const invite = flow.invite;
    if (invite === null) return null;
    const used_by = live.journal
      .filter(
        (e) => e.kind === 'device_joined' && e.network === networkId && e.at_ms >= flow.createdAtMs,
      )
      .map((e) => (e.kind === 'device_joined' ? e.name : ''));
    const listed = live.invites[networkId]?.find((i) => i.id === invite.id);
    return { ...invite, used_by, uses_left: listed ? listed.uses_left : invite.uses };
  });

  /** Every use taken: the QR has nothing left to offer, so it collapses for good. */
  const spent = $derived(
    current !== null &&
      current.uses !== null &&
      (current.uses_left === 0 || current.used_by.length >= current.uses),
  );

  /** Set once the invite's time is up; a timer flips it at the expiry instant. */
  let expiredSeen = $state<string | null>(null);
  const expired = $derived(current !== null && expiredSeen === current.link);
  // The parent's footer offers a new invite once this one is dead.
  $effect(() => {
    flow.expired = expired;
  });
  $effect(() => {
    const invite = current;
    if (invite === null) return;
    const left = invite.expires_at_ms - live.now();
    if (left <= 0) {
      expiredSeen = invite.link;
      return;
    }
    const timer = setTimeout(() => (expiredSeen = invite.link), Math.min(left, 2 ** 31 - 1));
    return () => clearTimeout(timer);
  });

  /** `online · direct` for a device that is online with a path, else null. */
  function onlinePath(name: string): string | null {
    const peer = Object.values(live.peers[networkId] ?? {}).find((p) => p.name === name);
    if (!peer?.online) return null;
    const numbers = live.numbers[`${networkId}/${peer.node}`];
    return pathText(peer.path, deviceName(networkId, numbers?.relay ?? null));
  }

  function statusOf(invite: Invite & { uses_left: number | null }): string {
    const user = invite.used_by.at(-1);
    if (user === undefined) return copy.invite.notUsed(clock(invite.expires_at_ms));
    const path = onlinePath(user);
    if (invite.uses === 1) {
      return path !== null ? copy.invite.usedBy(user, path) : copy.invite.usedOffline(user);
    }
    const left =
      invite.uses === null
        ? null
        : Math.max(0, Math.min(invite.uses_left ?? 0, invite.uses - invite.used_by.length));
    return copy.invite.usedByMany(
      invite.used_by,
      left,
      path !== null ? copy.invite.latestOnline(user, path) : null,
    );
  }

  const status = $derived.by(() => {
    if (current === null) return null;
    if (expired) return copy.invite.expired(clock(current.expires_at_ms));
    return statusOf(current);
  });

  // Each use hides the QR again; the reveal stays while uses remain.
  let qrShown = $state(false);
  let usesSeen = 0;
  $effect(() => {
    const count = current?.used_by.length ?? 0;
    if (count > usesSeen) qrShown = false;
    usesSeen = count;
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

  function revoke(): void {
    if (stale) return;
    const undo = flow.revoke((error) => toast.failed(failureText(error)));
    if (undo !== null) toast.done(copy.invite.revoked, { undo });
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
        disabled={flow.pending || stale}
        onValueChange={(v) => (flow.validity = v as Validity)}
      />
    </div>
    <div class="choice">
      <p class="label" aria-hidden="true">{copy.invite.uses}</p>
      <Segmented
        label={copy.invite.uses}
        options={usesOptions}
        value={flow.uses}
        disabled={flow.pending || stale}
        onValueChange={(v) => (flow.uses = v as UsesChoice)}
      />
    </div>
    {#if createFailed}<p class="failed">{createFailed}</p>{/if}
  </div>
{:else}
  <div class="result">
    {#if !expired}
      <CopyField value={flow.invite.link} label={copy.invite.link} />
    {/if}
    <QrPanel svg={flow.invite.qr_svg} collapse={spent || expired} bind:shown={qrShown} />
    {#if !expired}
      <div class="run">
        <p class="label">{copy.invite.orRun}</p>
        <CopyField value={copy.invite.runCommand(flow.invite.link)} label={copy.invite.orRun} />
      </div>
    {/if}
    <p class="status" class:over={expired} role="status">{status ?? ''}</p>
    {#if !expired}
      <div class="revoke">
        <Button variant="danger-quiet" disabled={stale} onclick={revoke}>
          {copy.invite.revoke}
        </Button>
        {#if stale}<p class="reason">{copy.conn.staleActions}</p>{/if}
      </div>
    {/if}
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
    display: grid;
    gap: var(--s-1);
  }
  .status {
    color: var(--ink-1);
  }
  .status.over {
    color: var(--ink-3);
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
