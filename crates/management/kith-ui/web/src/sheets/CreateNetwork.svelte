<!--
@component
CreateNetwork (spec §7.1): name the network, then this device becomes its first device and
its admin. The name is prefilled and selected (`home`); capitals lower as you type and a
space becomes a hyphen with a 1 s note. Validation runs on blur and, debounced, as you
type, through /api/validate/name; it never disables the button. The helper shows only for a
usable name. Success waits until the status lists the new network, then the sheet offers
the next device: Laptop or server, Phone or tablet, or Later (Overview).
Props:
  - open: boolean, bindable. The screen maps it to the URL query (Back closes it).
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { announce } from '../lib/announce';
  import { copy } from '../lib/copy';
  import { actions, live } from '../lib/live/store.svelte';
  import { SPACES_HINT_MS, VALIDATE_DEBOUNCE_MS } from '../lib/live/timing';
  import type { ActionHandle } from '../lib/types';
  import type { NameCheck, Outcome } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import { host as thisHost } from '../frame/host.svelte';
  import { go, href } from '../router';
  import { failureText, isStale, nameProblemText, normaliseName } from './flow';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  let name = $state<string>(copy.createNetwork.initial);
  let error = $state<string | null>(null);
  let hint = $state<string | null>(null);
  /** A free name close to a taken one, offered under the field. */
  let suggestion = $state<string | null>(null);
  /** The name the daemon accepted, until the status lists it. */
  let awaiting = $state<string | null>(null);
  /** The name that was created, once the status lists it. */
  let created = $state<string | null>(null);
  let handle = $state<ActionHandle<Outcome> | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let firstChoice = $state<HTMLButtonElement | null>(null);
  let checking = 0;
  let submitting = false;
  let hintTimer: ReturnType<typeof setTimeout> | null = null;
  let checkTimer: ReturnType<typeof setTimeout> | null = null;

  const host = $derived(thisHost.name);
  const creating = $derived(handle?.state === 'pending' || awaiting !== null);
  const failed = $derived(handle?.state === 'failed' && handle.error ? failureText(handle.error) : null);
  const stale = $derived(isStale());

  $effect(() => {
    if (failed) announce(failed);
  });

  // Each time the sheet opens it starts fresh, with `home` selected.
  $effect(() => {
    if (!open) return;
    name = copy.createNetwork.initial;
    error = null;
    hint = null;
    suggestion = null;
    awaiting = null;
    created = null;
    handle = null;
    void tick().then(() => input?.select());
  });

  // The new network is ready once the status lists it; only then can the page open it.
  $effect(() => {
    const value = awaiting;
    if (value === null) return;
    if (!(live.status?.networks ?? []).some((n) => n.name === value)) return;
    awaiting = null;
    created = value;
    announce(copy.createNetwork.ready(value));
    void tick().then(() => firstChoice?.focus());
  });

  function takenLocally(value: string): boolean {
    return (live.status?.networks ?? []).some((n) => n.name === value);
  }

  /**
   * Runs the shared validator and shows its answer, unless a newer check started meanwhile.
   * Returns the error now showing, or null when the name is usable.
   */
  async function validate(value: string): Promise<string | null> {
    const run = ++checking;
    const taken = copy.createNetwork.taken(value);
    let next: string | null = null;
    let offer: string | null = null;
    const result: NameCheck | null = (await actions.validateName(null, value).done).result;
    if (result?.problem) {
      next = nameProblemText(result.problem, taken);
      offer = result.suggestion;
    } else if (takenLocally(value)) {
      next = taken;
    }
    if (run === checking) {
      error = next;
      suggestion = offer;
    }
    return error;
  }

  function onInput(event: Event & { currentTarget: HTMLInputElement }): void {
    const field = event.currentTarget;
    const caret = field.selectionStart;
    const { value, spaced } = normaliseName(field.value);
    if (value !== field.value) {
      field.value = value;
      if (caret !== null) field.setSelectionRange(caret, caret);
    }
    name = value;
    if (spaced) {
      hint = copy.createNetwork.spaces;
      if (hintTimer !== null) clearTimeout(hintTimer);
      hintTimer = setTimeout(() => (hint = null), SPACES_HINT_MS);
    }
    if (checkTimer !== null) clearTimeout(checkTimer);
    checkTimer = setTimeout(() => void validate(value), VALIDATE_DEBOUNCE_MS);
  }

  function onBlur(): void {
    if (checkTimer !== null) clearTimeout(checkTimer);
    void validate(name);
  }

  function useSuggestion(value: string): void {
    name = value;
    error = null;
    suggestion = null;
    void tick().then(() => input?.focus());
  }

  async function submit(event?: SubmitEvent): Promise<void> {
    event?.preventDefault();
    if (submitting || stale || creating) return;
    if (checkTimer !== null) clearTimeout(checkTimer);
    submitting = true;
    const value = name;
    if ((await validate(value)) !== null) {
      submitting = false;
      input?.focus();
      return;
    }
    const next = actions.createNetwork(value, host);
    handle = next;
    const settled = await next.done;
    submitting = false;
    if (settled.state === 'accepted' || settled.state === 'seen') awaiting = value;
  }

  function later(): void {
    if (created === null) return;
    open = false;
    go(href({ name: 'overview', net: created }));
  }

  function addNext(kind: 'device' | 'phone'): void {
    if (created === null) return;
    open = false;
    go(`${href({ name: 'overview', net: created })}?add=${kind}`);
  }

  // Closing after success still lands on the new network, never back on Welcome.
  function onOpenChange(next: boolean): void {
    if (!next && created !== null) go(href({ name: 'overview', net: created }));
  }
</script>

<Sheet bind:open title={copy.createNetwork.title} closeLabel={copy.action.close} {onOpenChange}>
  {#if created === null}
    <form id="create-network" class="form" onsubmit={submit} novalidate>
      <Field
        label={copy.createNetwork.label}
        helper={error === null && name !== '' ? copy.createNetwork.helper(host, name) : undefined}
        {error}
        {hint}
      >
        {#snippet children(control)}
          <Input
            {...control}
            bind:ref={input}
            value={name}
            kind="name"
            oninput={onInput}
            onblur={onBlur}
          />
        {/snippet}
      </Field>
      {#if error !== null && suggestion !== null}
        {@const offer = suggestion}
        <div class="alt">
          <Button variant="ghost" onclick={() => useSuggestion(offer)}>
            {copy.createNetwork.useSuggestion(offer)}
          </Button>
        </div>
      {/if}
    </form>
  {:else}
    <div class="done">
      <p class="t-heading">{copy.createNetwork.ready(created)}</p>
      <p class="next">{copy.createNetwork.next}</p>
      <div class="choices">
        <Button variant="secondary" bind:ref={firstChoice} onclick={() => addNext('device')}>
          {copy.createNetwork.laptop}
        </Button>
        <Button variant="secondary" onclick={() => addNext('phone')}>
          {copy.createNetwork.phone}
        </Button>
      </div>
    </div>
  {/if}

  {#snippet footer()}
    {#if created === null}
      <div class="note">
        {#if stale}
          <p class="reason">{copy.conn.staleActions}</p>
        {:else if failed}
          <p class="failed">{failed}</p>
        {/if}
      </div>
      <Button
        variant="primary"
        type="submit"
        form="create-network"
        disabled={stale}
        pending={creating}
        pendingLabel={copy.createNetwork.pending}
      >
        {name === '' ? copy.createNetwork.submitEmpty : copy.createNetwork.submit(name)}
      </Button>
    {:else}
      <Button variant="ghost" onclick={later}>{copy.createNetwork.later}</Button>
    {/if}
  {/snippet}
</Sheet>

<style>
  .form {
    display: grid;
    gap: var(--s-2);
  }
  .alt {
    margin-inline-start: calc(-1 * (var(--s-3) + var(--hairline)));
  }
  .alt :global(.btn) {
    color: var(--ink-2);
    font-weight: 400;
  }
  .done {
    display: grid;
    gap: var(--s-3);
    max-width: var(--measure);
  }
  .next {
    margin-top: var(--s-3);
    color: var(--ink-2);
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3);
  }
  .note {
    flex: 1;
    min-width: 0;
    align-self: center;
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
