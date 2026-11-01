<!--
@component
Welcome (spec §6.1): this device is in no network yet. No app frame; a centred column,
max 640 px. `kith`, the one-line promise, then two large card buttons (side by side on
desktop, stacked on phones): Start a network and Join a network, each opening its sheet
through `?flow=start|join` so Back closes it. The footer names this device, with Rename:
an inline field validated like any name (Enter saves, Esc reverts).
Props:
  - flow: string | null, the `?flow=` value.
-->
<script lang="ts">
  import { host } from '../frame/host.svelte';
  import { nameProblem } from '../lib/kith/errors';
  import { copy } from '../lib/copy';
  import { actions } from '../lib/live/store.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import { closeOverlay, openOverlay } from '../router';
  import CreateNetwork from '../sheets/CreateNetwork.svelte';
  import JoinNetwork from '../sheets/JoinNetwork.svelte';

  let { flow }: { flow: string | null } = $props();

  let editing = $state(false);
  let draft = $state('');
  let error = $state<string | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let renameButton = $state<HTMLButtonElement | null>(null);
  let checking = $state(false);

  function edit(): void {
    draft = host.name;
    error = null;
    editing = true;
    queueMicrotask(() => input?.select());
  }

  function stop(): void {
    editing = false;
    error = null;
    queueMicrotask(() => renameButton?.focus());
  }

  async function check(): Promise<boolean> {
    const handle = actions.validateName(null, draft);
    checking = true;
    await handle.done;
    checking = false;
    const result = handle.result;
    if (!result) return true;
    error = result.problem === null ? null : nameProblem(result.problem);
    if (result.problem === null) draft = result.name;
    return result.problem === null;
  }

  async function save(): Promise<void> {
    if (!(await check())) return;
    host.name = draft;
    stop();
  }

  function keys(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      void save();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      stop();
    }
  }

  const closeFlow = (open: boolean): void => {
    if (!open) closeOverlay('flow');
  };
</script>

<section class="welcome" aria-labelledby="welcome-title">
  <header class="intro">
    <h1 id="welcome-title" class="brand">{copy.welcome.title}</h1>
    <p class="lede">{copy.welcome.lede}</p>
  </header>

  <div class="choices">
    <button type="button" class="choice" onclick={() => openOverlay('flow', 'start')}>
      <span class="choice-title">{copy.welcome.start}</span>
      <span class="choice-sub">{copy.welcome.startSub}</span>
    </button>
    <button type="button" class="choice" onclick={() => openOverlay('flow', 'join')}>
      <span class="choice-title">{copy.welcome.join}</span>
      <span class="choice-sub">{copy.welcome.joinSub}</span>
    </button>
  </div>

  <footer class="device">
    {#if editing}
      <div class="rename">
        <Field label={copy.welcome.nameLabel} {error}>
          {#snippet children(control)}
            <Input
              {...control}
              kind="name"
              bind:value={draft}
              bind:ref={input}
              onkeydown={keys}
              onblur={() => void check()}
              oninput={() => error !== null && void check()}
            />
          {/snippet}
        </Field>
        <div class="rename-actions">
          <Button variant="ghost" onclick={stop}>{copy.action.cancel}</Button>
          <Button pending={checking} onclick={() => void save()}>{copy.welcome.save}</Button>
        </div>
      </div>
    {:else}
      <p>{copy.welcome.thisDevice(host.name, host.os)}</p>
      <Button variant="ghost" bind:ref={renameButton} onclick={edit}>{copy.welcome.rename}</Button>
    {/if}
  </footer>
</section>

<CreateNetwork bind:open={() => flow === 'start', closeFlow} />
<JoinNetwork bind:open={() => flow === 'join', closeFlow} />

<style>
  .welcome {
    display: grid;
    align-content: center;
    gap: var(--s-7);
    min-height: 100dvh;
    max-width: var(--welcome-max);
    margin: 0 auto;
    padding: var(--s-8) var(--gutter-mobile);
  }
  .intro {
    display: grid;
    gap: var(--s-2);
  }
  .brand {
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
  }
  .lede {
    color: var(--ink-2);
    max-width: var(--measure);
  }
  .choices {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--s-4);
  }
  .choice {
    display: grid;
    align-content: start;
    gap: var(--s-2);
    padding: var(--s-5);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface);
    text-align: left;
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .choice:hover {
    background: var(--sunken);
  }
  .choice-title {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
    color: var(--ink-1);
  }
  .choice-sub {
    color: var(--ink-2);
  }
  .device {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-3);
  }
  .rename {
    display: grid;
    gap: var(--s-3);
    width: 100%;
  }
  .rename-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-3);
  }
  @media (max-width: 40rem) {
    .choices {
      grid-template-columns: 1fr;
    }
  }
</style>
