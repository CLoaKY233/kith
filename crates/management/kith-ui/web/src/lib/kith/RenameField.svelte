<!--
@component
RenameField (spec §6.4 header, §7.11, tier 0): a device's name at `--t-title` that an admin
edits in place. Click the name or the pencil (always visible on touch); it becomes a field
with the CLI's validation and a live DNS preview. Enter saves, Esc reverts. Saving is
instant with Undo; a name another admin changed meanwhile shows both and lets the person
pick.
Props:
  - net: string, the network's id or name.
  - node: string, the device's id.
  - editable: boolean. False for members: the name is plain text (they ask an admin from
    the detail's actions instead, §6.4).
  - heading?: 'h1' | 'h2' (default 'h1'), the element that holds the name.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { announce } from '../announce';
  import { copy } from '../copy';
  import { actions, live } from '../live/store.svelte';
  import { SPACES_HINT_MS, UNDO_MS } from '../live/timing';
  import type { ActionHandle } from '../types';
  import type { Outcome } from '../types.gen';
  import Button from '../ui/Button.svelte';
  import Field from '../ui/Field.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Input from '../ui/Input.svelte';
  import { toast } from '../ui/Toast.svelte';
  import {
    cannotAct,
    failureText,
    fullySeen,
    nameProblemText,
    namesPointingAt,
    networkContext,
    normalizeInput,
    progressText,
  } from '../../sheets/act';

  let {
    net,
    node,
    editable,
    heading = 'h1',
  }: { net: string; node: string; editable: boolean; heading?: 'h1' | 'h2' } = $props();

  type Race = { who: string; from: string; to: string; op: string | null };

  const context = $derived(networkContext(live.status, net));
  const netId = $derived(context?.id ?? net);
  const networkName = $derived(context?.name ?? net);
  const peer = $derived(live.peers[netId]?.[node] ?? null);
  const self = $derived(node === live.status?.node);
  const name = $derived(peer?.name ?? (self ? (context?.membership?.name ?? '') : ''));
  const address = $derived(
    peer?.address ?? (self ? (context?.membership?.address ?? '') : ''),
  );
  const stale = $derived(cannotAct(live.conn));

  let editing = $state(false);
  let value = $state('');
  /** The name when editing began, for Esc, Undo and the concurrent-edit band. */
  let original = $state('');
  let error = $state<string | null>(null);
  let suggestion = $state<string | null>(null);
  let spacesHint = $state(false);
  let race = $state<Race | null>(null);
  let saving = $state.raw<ActionHandle<Outcome> | null>(null);
  let saved = $state.raw<ActionHandle<Outcome> | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let pencil = $state<HTMLElement | null>(null);

  const pending = $derived(saving?.state === 'pending');
  const pointing = $derived(
    namesPointingAt(live.records[netId] ?? [], { name: original, address }, networkName),
  );
  const progress = $derived(saved && !fullySeen(saved) ? progressText(saved) : null);

  async function start(): Promise<void> {
    if (!editable) return;
    original = name;
    value = name;
    error = null;
    suggestion = null;
    race = null;
    saving = null;
    editing = true;
    if (!live.records[netId]) actions.loadRecords(netId);
    await tick();
    input?.select();
  }

  async function stop(): Promise<void> {
    editing = false;
    race = null;
    await tick();
    pencil?.querySelector('button')?.focus();
  }

  let checkSeq = 0;

  /** Checks the name with the CLI's validator; only the latest answer counts. */
  async function validate(): Promise<boolean> {
    if (value === original) {
      error = null;
      suggestion = null;
      return true;
    }
    const seq = ++checkSeq;
    const settled = await actions.validateName(netId, value).done;
    if (seq !== checkSeq) return false;
    const check = settled.result;
    if (!check) return true;
    error = nameProblemText(check, networkName);
    suggestion = check.problem === 'taken' ? check.suggestion : null;
    return error === null;
  }

  let hintTimer: ReturnType<typeof setTimeout> | null = null;

  function typed(event: Event & { currentTarget: HTMLInputElement }): void {
    const next = normalizeInput(event.currentTarget);
    value = next.value;
    if (next.spaced) {
      spacesHint = true;
      if (hintTimer !== null) clearTimeout(hintTimer);
      hintTimer = setTimeout(() => (spacesHint = false), SPACES_HINT_MS);
    }
    if (error !== null) void validate();
  }

  function keydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    void stop();
  }

  function useSuggestion(next: string): void {
    value = next;
    error = null;
    suggestion = null;
    input?.focus();
  }

  async function save(): Promise<void> {
    if (stale || pending) return;
    if (value === original) {
      void stop();
      return;
    }
    if (!(await validate())) {
      input?.focus();
      return;
    }
    await run(null);
  }

  async function run(basedOn: string | null): Promise<void> {
    const from = original;
    const to = value;
    const handle = actions.renameDevice(netId, node, to, basedOn);
    saving = handle;
    race = null;
    const settled = await handle.done;
    if (saving !== handle) return;
    if (settled.state === 'conflict' && settled.conflict) {
      race = { who: settled.conflict.resolved_by, from, to: name, op: settled.conflict.op };
      announce(raceText(race));
      return;
    }
    if (settled.state === 'failed' && settled.error) {
      error =
        settled.error.code === 'invalid_argument'
          ? copy.error.nameChars
          : failureText(settled.error, from);
      announce(error);
      return;
    }
    saved = handle;
    await stop();
    const done = copy.rename.done(from, to);
    announce(done);
    toast.done(done, {
      undoMs: UNDO_MS,
      undo: () => {
        saved = actions.renameDevice(netId, node, from);
      },
    });
  }

  function raceText(r: Race): string {
    return r.to !== r.from
      ? copy.rename.concurrent(r.who, r.from, r.to)
      : copy.error.conflict(r.who);
  }

  function useTheirs(): void {
    void stop();
  }

  function keepMine(): void {
    void run(race?.op ?? null);
  }
</script>

<div class="rename">
  {#if editing}
    <form
      class="edit"
      onsubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <Field
        label={copy.names.name}
        helper={copy.rename.preview(value || original, networkName)}
        {error}
        hint={spacesHint ? copy.createNetwork.spaces : null}
      >
        {#snippet children(control)}
          <Input
            {...control}
            kind="name"
            bind:value
            bind:ref={input}
            readonly={pending}
            oninput={typed}
            onblur={() => void validate()}
            onkeydown={keydown}
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
      {#if pointing > 0}
        <p class="note">{copy.rename.follow(pointing, original)}</p>
      {/if}
      {#if race}
        <div class="band">
          <p>{raceText(race)}</p>
          <div class="band-buttons">
            <Button onclick={useTheirs}>{copy.rename.useTheirs}</Button>
            <Button disabled={stale} onclick={keepMine}>{copy.rename.keepMine}</Button>
          </div>
        </div>
      {/if}
      {#if pending}
        <p class="note">{copy.action.signing}</p>
      {/if}
      {#if stale}
        <p class="note">{copy.conn.staleActions}</p>
      {/if}
    </form>
  {:else}
    <div class="show">
      <svelte:element this={heading} class="t-title name">
        {#if editable}
          <button type="button" class="name-button" tabindex="-1" onclick={() => void start()}>
            {name}
          </button>
        {:else}
          {name}
        {/if}
      </svelte:element>
      {#if editable}
        <span class="pencil" bind:this={pencil}>
          <IconButton
            icon="pencil"
            label={copy.palette.rename(name)}
            onclick={() => void start()}
          />
        </span>
      {/if}
    </div>
    {#if progress}
      <p class="note">{progress}</p>
    {/if}
  {/if}
</div>

<style>
  .rename {
    display: grid;
    gap: var(--s-1);
  }
  .edit {
    display: grid;
    gap: var(--s-3);
    max-width: var(--detail);
  }
  .show {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
  }
  .name {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .name-button {
    font: inherit;
    letter-spacing: inherit;
    color: var(--ink-1);
    text-align: start;
    cursor: text;
  }
  .pencil {
    opacity: 0;
    transition: opacity var(--d-hover) var(--e-linear);
  }
  .show:hover .pencil,
  .pencil:focus-within {
    opacity: 1;
  }
  @media (pointer: coarse) {
    .pencil {
      opacity: 1;
    }
  }
  .note {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .band {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-3);
    border-radius: var(--r-2);
    background: var(--attn-wash);
    color: var(--attn-text);
  }
  .band-buttons {
    display: flex;
    gap: var(--s-3);
  }
</style>
