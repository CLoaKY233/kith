<!--
@component
AllowBlocked (spec §7.5, tier 1): `Allow this…` on a blocked-connection card. Shows the
one rule that will be signed and what else it opens, then signs it on `Add rule`. Never
one click (§0 ruling 5). A member gets the Ask-an-admin body instead (§7.10).

Opens as a popover anchored to the card's button on desktop when `trigger` is given, and as
a sheet otherwise (mobile, `?need=`, the device verdict's `Allow this…`).

Props:
  - open?: boolean, bindable. The screen maps it to the URL query (Back closes it).
  - id: string, the BlockedGroup id.
  - trigger?: Snippet<[Record<string, unknown>]>, the card's `Allow this…` button; spread
    the props on it. On mobile it still renders and opens the sheet.
  - onOpenChange?: (open: boolean) => void.

Module exports, for the card (NeedsYouCard, kind `blocked`):
  - ignoreBlocked(id, nowMs): `Ignore`. Hides the group for a day and says so, with Undo.
  - isBlockedIgnored(id, nowMs): whether the card should stay hidden.
  - allowRequest(group, network): the request a member sends an admin.
-->
<script lang="ts" module>
  import { SvelteMap } from 'svelte/reactivity';
  import { copy as text } from '../lib/copy';
  import { IGNORE_BLOCKED_MS, UNDO_MS } from '../lib/live/timing';
  import type { BlockedGroup } from '../lib/types.gen';
  import { toast } from '../lib/ui/Toast.svelte';

  const IGNORED_KEY = 'kith.blocked.ignored';

  function readIgnored(): [string, number][] {
    try {
      const raw = localStorage.getItem(IGNORED_KEY);
      const parsed: unknown = raw ? JSON.parse(raw) : null;
      if (!parsed || typeof parsed !== 'object') return [];
      const now = Date.now();
      return Object.entries(parsed).filter(
        (pair): pair is [string, number] => typeof pair[1] === 'number' && pair[1] > now,
      );
    } catch {
      return [];
    }
  }

  /** Ignored group ids and when each comes back, in this viewer's browser only. */
  const ignored = new SvelteMap<string, number>(readIgnored());

  function saveIgnored(): void {
    try {
      localStorage.setItem(IGNORED_KEY, JSON.stringify(Object.fromEntries(ignored)));
    } catch {
      // Private window or blocked storage: it stays hidden for this visit only.
    }
  }

  export function ignoreBlocked(id: string, nowMs: number): void {
    ignored.set(id, nowMs + IGNORE_BLOCKED_MS);
    saveIgnored();
    toast.done(text.blocked.ignored, {
      undoMs: UNDO_MS,
      undo: () => {
        ignored.delete(id);
        saveIgnored();
      },
    });
  }

  export function isBlockedIgnored(id: string, nowMs: number): boolean {
    const until = ignored.get(id);
    return until !== undefined && until > nowMs;
  }

  export function allowRequest(group: BlockedGroup, network: string): string {
    return text.askAdmin.request.allow(
      network,
      group.from_name,
      group.to_name,
      group.protocol,
      group.port,
    );
  }
</script>

<script lang="ts">
  import { untrack, type Snippet } from 'svelte';
  import { MediaQuery } from 'svelte/reactivity';
  import { announce } from '../lib/announce';
  import { copy, DOT } from '../lib/copy';
  import AskAdminSheet from '../lib/kith/AskAdminSheet.svelte';
  import { actions, live } from '../lib/live/store.svelte';
  import type { ActionHandle } from '../lib/types';
  import type { AllowPreview, Outcome } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import Popover from '../lib/ui/Popover.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import {
    cannotAct,
    failureText,
    fullySeen,
    networkContext,
    progressText,
  } from './act';

  let {
    open = $bindable(false),
    id,
    trigger,
    onOpenChange,
  }: {
    open?: boolean;
    id: string;
    trigger?: Snippet<[Record<string, unknown>]>;
    onOpenChange?: (open: boolean) => void;
  } = $props();

  const narrow = new MediaQuery('(max-width: 40rem)');
  const asPopover = $derived(trigger !== undefined && !narrow.current);

  const current = $derived(live.pending.blocked.find((g) => g.id === id) ?? null);
  /** Kept once seen, so the outcome still reads after the group leaves Needs you. */
  let held = $state.raw<BlockedGroup | null>(null);
  $effect(() => {
    if (current) held = current;
  });
  const group = $derived(current ?? (held?.id === id ? held : null));

  const network = $derived(group ? networkContext(live.status, group.network) : null);
  const admin = $derived(network?.admin ?? false);
  const stale = $derived(cannotAct(live.conn));

  let preview = $state.raw<ActionHandle<AllowPreview> | null>(null);
  let signing = $state.raw<ActionHandle<Outcome> | null>(null);

  $effect(() => {
    if (!open) {
      preview = null;
      signing = null;
      return;
    }
    if (admin && group && preview === null && !stale) preview = actions.previewAllow(id);
  });

  /** The rule first: from the card on the first frame, then the server's own sentence. */
  const rule = $derived.by(() => {
    if (preview?.result) return copy.blocked.ruleSentence(preview.result.rule_sentence);
    if (!group || group.port === null) return null;
    return copy.blocked.rule(
      group.from_name,
      group.to_name,
      group.protocol.toUpperCase(),
      group.port,
    );
  });

  const paths = $derived.by(() => {
    const result = preview?.result;
    if (!result) return null;
    return result.new_paths.length <= 1
      ? copy.blocked.nothingElse
      : copy.blocked.opens(result.new_paths.length, result.new_paths.join(DOT));
  });

  const done = $derived(
    group === null
      ? ''
      : group.port === null
        ? copy.blocked.doneAny(group.from_name, group.to_name)
        : copy.blocked.done(group.from_name, group.to_name, group.port),
  );

  /** §7.5 step 3: `Allowed by you · spreading 3 of 5`, then `phone can reach nas:22 now.` */
  const outcome = $derived.by(() => {
    if (!signing || !group) return null;
    if (signing.state === 'conflict' && signing.conflict) {
      return copy.error.conflict(signing.conflict.resolved_by);
    }
    if (signing.state === 'failed' && signing.error) {
      return failureText(signing.error, group.to_name);
    }
    if (signing.state === 'pending') return null;
    if (fullySeen(signing)) return done;
    return progressText(signing, copy.blocked.spreading);
  });

  const signed = $derived(
    signing !== null && (signing.state === 'accepted' || signing.state === 'seen'),
  );
  /** Signed here, or by another admin first (a 409 shows their result, not an error). */
  const settled = $derived(signed || signing?.state === 'conflict');

  /** Announces the first result and the final one, not every count in between (§11.1). */
  let announced = $state<'none' | 'signed' | 'done'>('none');
  $effect(() => {
    if (!signed || !outcome) {
      if (!signing) announced = 'none';
      return;
    }
    const final = outcome === done;
    if (announced === 'done' || (announced === 'signed' && !final)) return;
    announce(outcome);
    announced = final ? 'done' : 'signed';
  });

  /** Reports every open and close, whichever layer made it, so the URL can follow. */
  let last: boolean | null = null;
  $effect(() => {
    const now = open;
    if (last !== null && now !== last) untrack(() => onOpenChange?.(now));
    last = now;
  });

  function addRule(): void {
    if (stale || !group) return;
    signing = actions.allowBlocked(id);
  }

  function close(): void {
    open = false;
  }
</script>

{#snippet body()}
  {#if !group}
    <p class="note">{copy.blocked.gone}</p>
  {:else if !admin}
    <AskAdminSheet
      bare
      net={group.network}
      request={allowRequest(group, network?.name ?? group.network)}
    />
  {:else}
    <div class="allow">
      {#if !asPopover}
        <p class="context">
          {copy.blocked.card(
            group.from_name,
            group.to_name,
            group.port === null ? '' : copy.blocked.port(group.port, null),
            group.count,
          )}
        </p>
      {/if}
      <p class="rule">{rule ?? ''}</p>
      {#if preview?.error}
        <p class="fail">{failureText(preview.error, group.to_name)}</p>
      {:else}
        <p class="paths">{paths ?? ''}</p>
      {/if}
      {#if outcome}
        <p class="outcome" class:fail={signing?.state === 'failed'}>{outcome}</p>
      {/if}
      {#if stale}
        <p class="note">{copy.conn.staleActions}</p>
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet buttons()}
  {#if !group || !admin || settled}
    <Button onclick={close}>{copy.action.close}</Button>
  {:else}
    <Button onclick={close}>{copy.action.cancel}</Button>
    <Button
      variant="primary"
      disabled={stale}
      pending={signing?.state === 'pending'}
      onclick={addRule}
    >
      {copy.blocked.addRule}
    </Button>
  {/if}
{/snippet}

{#if asPopover && trigger}
  <Popover bind:open {trigger} width="card" label={copy.blocked.allowThis}>
    <div class="pop">
      {@render body()}
      <div class="pop-buttons">{@render buttons()}</div>
    </div>
  </Popover>
{:else}
  {#if trigger}
    {@render trigger({ onclick: () => (open = true), 'aria-haspopup': 'dialog' })}
  {/if}
  <Sheet
    bind:open
    title={admin || !group ? copy.blocked.allowThis : copy.askAdmin.title}
    closeLabel={copy.action.close}
  >
    {@render body()}
    {#snippet footer()}{@render buttons()}{/snippet}
  </Sheet>
{/if}

<style>
  .allow {
    display: grid;
    gap: var(--s-2);
    max-width: var(--measure);
  }
  .context {
    color: var(--ink-2);
  }
  .rule {
    min-height: 1lh;
  }
  .paths {
    min-height: 1lh;
    color: var(--ink-2);
  }
  .outcome {
    color: var(--ink-1);
  }
  .note {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .fail {
    color: var(--danger);
  }
  .pop {
    display: grid;
    gap: var(--s-4);
  }
  .pop-buttons {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-3);
  }
</style>
