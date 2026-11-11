<!--
@component
Setup (docs/plans/console.md, flow 1): the one-time `/setup/<token>` page, before anyone
can sign in. No app frame, centred, max 520 px. Three steps with the progress shown:
names (network and person), the passkey (create it, then confirm the owner with Touch ID),
and the recovery words, shown once until `I saved these`. Every failure says why in words:
no passkey support, a closed prompt, an expired or used link.
Props:
  - token: string, from the URL.
-->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { accounts } from '../lib/accounts';
  import { ApiError } from '../lib/api';
  import { copy } from '../lib/copy';
  import { copyConsole } from '../lib/copy.console';
  import { nameProblem } from '../lib/kith/errors';
  import {
    initialSetup,
    networkNameProblem,
    SETUP_STEPS,
    setupBusy,
    setupProblem,
    setupStep,
    setupStepNumber,
    type SetupEvent,
    type SetupProblem,
    type SetupState,
  } from '../lib/setup';
  import Button from '../lib/ui/Button.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Input from '../lib/ui/Input.svelte';
  import type { SetupInfo } from '../lib/types.gen';
  import { createPasskey, passkeysSupported, usePasskey } from '../lib/webauthn';
  import { normaliseName } from '../sheets/flow';

  let { token }: { token: string } = $props();

  let wizard = $state<SetupState>(initialSetup(passkeysSupported()));
  /** What the link says: whether this device is in a network already. Null until read. */
  let info = $state.raw<SetupInfo | null>(null);
  /** The network setup names: the one this device is in, else what the person types. */
  let network = $state('home');
  let person = $state('');
  let networkError = $state<string | null>(null);
  let personError = $state<string | null>(null);
  let heading = $state<HTMLElement | null>(null);

  const step = $derived(setupStepNumber(wizard));
  const busy = $derived(setupBusy(wizard));

  function send(event: SetupEvent): void {
    const before = setupStepNumber(wizard);
    wizard = setupStep(wizard, event);
    // A new step moves focus to its heading, so a screen reader hears where it is.
    if (setupStepNumber(wizard) !== before) void tick().then(() => heading?.focus());
  }

  function fail(cause: unknown): void {
    send({ type: 'failed', problem: setupProblem(cause) });
  }

  onMount(() => {
    if (wizard.phase === 'blocked') return;
    accounts.setupInfo(token).then(
      (answer) => {
        info = answer;
        if (answer.network) network = answer.network.name;
      },
      (cause: unknown) => {
        fail(cause);
        // Not knowing is no reason to stop: the server ignores a network name it has.
        info = { network: null, expires_at_ms: 0 };
      },
    );
  });

  /** The network exists already, so setup only names its owner. */
  const named = $derived(info?.network != null);

  function checkNetwork(): boolean {
    if (named) return true;
    const problem = networkNameProblem(network);
    networkError = problem === null ? null : nameProblem(problem);
    return problem === null;
  }

  function checkPerson(): boolean {
    personError = person.trim() === '' ? copyConsole.setup.personEmpty : null;
    return personError === null;
  }

  function onNetworkInput(event: Event & { currentTarget: HTMLInputElement }): void {
    network = normaliseName(event.currentTarget.value).value;
    if (networkError !== null) checkNetwork();
  }

  async function start(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    const ok = [checkNetwork(), checkPerson()].every(Boolean);
    if (!ok) return;
    send({ type: 'start' });
    try {
      const answer = await accounts.setupBegin(token, person.trim(), named ? null : network);
      send({ type: 'started', options: answer.public_key });
    } catch (cause) {
      fail(cause);
    }
  }

  async function create(): Promise<void> {
    if (wizard.phase !== 'passkey') return;
    const options = wizard.options;
    send({ type: 'create' });
    try {
      const credential = await createPasskey(options);
      const { authority } = await accounts.setupFinish(token, credential);
      send({ type: 'created', authority: authority.public_key });
    } catch (cause) {
      fail(cause);
    }
  }

  /** The recovery words, shown once; none when the network already had a recovery code. */
  async function recoveryWords(): Promise<readonly string[]> {
    try {
      return (await accounts.recovery()).words;
    } catch (cause) {
      if (cause instanceof ApiError && cause.code === 'conflict') return [];
      throw cause;
    }
  }

  async function confirm(): Promise<void> {
    if (wizard.phase !== 'confirm') return;
    const authority = wizard.authority;
    send({ type: 'confirm' });
    try {
      const assertion = await usePasskey(authority);
      await accounts.authorityFinish(assertion);
      send({ type: 'confirmed', words: await recoveryWords() });
    } catch (cause) {
      fail(cause);
    }
  }

  $effect(() => {
    if (wizard.phase === 'done') location.assign('/');
  });

  function problemText(problem: SetupProblem): string {
    switch (problem.kind) {
      case 'unsupported':
        return copyConsole.passkey.unsupported;
      case 'expired':
        return copyConsole.setup.expired;
      case 'used':
        return copyConsole.setup.used;
      case 'invalid':
        return copyConsole.setup.invalid;
      default:
        return problem.text;
    }
  }

  const problem = $derived(
    wizard.phase === 'names' || wizard.phase === 'passkey' || wizard.phase === 'confirm' || wizard.phase === 'blocked'
      ? wizard.problem
      : null,
  );
</script>

<section class="setup" aria-labelledby="setup-title">
  <header class="top">
    <p class="brand">{copy.brand}</p>
    <h1 id="setup-title" class="title">{copyConsole.setup.title}</h1>
    {#if step !== null}
      <ol class="steps" aria-label={copyConsole.setup.progress(step, SETUP_STEPS)}>
        {#each copyConsole.setup.steps as label, i (label)}
          <li class:done={i + 1 < step} class:current={i + 1 === step} aria-current={i + 1 === step ? 'step' : undefined}>
            {label}
          </li>
        {/each}
      </ol>
      <p class="meta">{copyConsole.setup.progress(step, SETUP_STEPS)}</p>
    {/if}
  </header>

  {#if wizard.phase === 'blocked'}
    <p class="problem" role="alert">{problemText(wizard.problem)}</p>
    {#if wizard.problem.kind === 'used'}
      <a class="link" href="/">{copyConsole.setup.signIn}</a>
    {/if}
  {:else if step === 1 && info === null}
    <p class="lede" role="status">{copyConsole.setup.checking}</p>
  {:else if step === 1}
    <form class="body" onsubmit={start} novalidate>
      <h2 class="heading" tabindex="-1" bind:this={heading}>
        {named ? copyConsole.setup.ownerHeading : copyConsole.setup.namesHeading}
      </h2>
      {#if named}
        <p class="lede">{copyConsole.setup.ownerLede(network)}</p>
      {:else}
        <p class="lede">{copyConsole.setup.namesLede}</p>
        <Field label={copyConsole.setup.networkLabel} helper={copyConsole.setup.networkHelper(network || 'home')} error={networkError}>
          {#snippet children(control)}
            <Input {...control} kind="name" value={network} oninput={onNetworkInput} onblur={checkNetwork} />
          {/snippet}
        </Field>
      {/if}
      <Field label={copyConsole.setup.personLabel} helper={copyConsole.setup.personHelper} error={personError}>
        {#snippet children(control)}
          <Input
            {...control}
            bind:value={person}
            autocomplete="name"
            onblur={checkPerson}
            oninput={() => personError !== null && checkPerson()}
          />
        {/snippet}
      </Field>
      {#if problem}<p class="problem" role="alert">{problemText(problem)}</p>{/if}
      <div class="actions">
        <Button variant="primary" type="submit" pending={wizard.phase === 'starting'}>{copyConsole.setup.continue}</Button>
      </div>
    </form>
  {:else if wizard.phase === 'passkey' || wizard.phase === 'creating'}
    <div class="body">
      <h2 class="heading" tabindex="-1" bind:this={heading}>{copyConsole.setup.passkeyHeading}</h2>
      <p class="lede">{copyConsole.setup.passkeyLede}</p>
      {#if problem}<p class="problem" role="alert">{problemText(problem)}</p>{/if}
      <div class="actions">
        {#if wizard.phase === 'passkey'}
          <Button variant="ghost" onclick={() => send({ type: 'back' })}>{copyConsole.setup.back}</Button>
        {/if}
        <Button variant="primary" pending={wizard.phase === 'creating'} pendingLabel={copyConsole.setup.creating} onclick={create}>
          {copyConsole.setup.createPasskey}
        </Button>
      </div>
    </div>
  {:else if wizard.phase === 'confirm' || wizard.phase === 'confirming'}
    <div class="body">
      <h2 class="heading" tabindex="-1">{copyConsole.setup.confirmHeading}</h2>
      <p class="lede">{copyConsole.setup.confirmLede(network)}</p>
      {#if problem}<p class="problem" role="alert">{problemText(problem)}</p>{/if}
      <div class="actions">
        <Button
          variant="primary"
          pending={wizard.phase === 'confirming'}
          pendingLabel={copyConsole.setup.confirmingLabel}
          onclick={confirm}
        >
          {copyConsole.setup.confirm}
        </Button>
      </div>
    </div>
  {:else if wizard.phase === 'words'}
    <div class="body">
      <h2 class="heading" tabindex="-1" bind:this={heading}>{copyConsole.setup.wordsHeading}</h2>
      <p class="lede">{copyConsole.setup.wordsLede}</p>
      <ol class="words" aria-label={copyConsole.setup.wordsLabel}>
        {#each wizard.words as word, i (i)}
          <li>{word}</li>
        {/each}
      </ol>
      <div class="actions">
        <Button variant="primary" onclick={() => send({ type: 'saved' })}>{copyConsole.setup.saved}</Button>
      </div>
    </div>
  {/if}
</section>

<style>
  .setup {
    display: grid;
    align-content: start;
    gap: var(--s-6);
    min-height: 100dvh;
    max-width: var(--gate-max);
    margin: 0 auto;
    padding: max(var(--s-8), 12dvh) var(--gutter-mobile) var(--s-8);
  }
  .top,
  .body {
    display: grid;
    gap: var(--s-4);
  }
  .top {
    gap: var(--s-2);
  }
  .brand {
    color: var(--ink-3);
  }
  .title {
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
  }
  .heading {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
    outline: none;
  }
  .lede {
    max-width: var(--measure);
    color: var(--ink-2);
  }
  .meta {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .steps {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--s-2);
    margin-top: var(--s-2);
    font: var(--t-small);
    color: var(--ink-3);
  }
  .steps li {
    padding-top: var(--s-2);
    border-top: var(--lead-bar) solid var(--line);
  }
  .steps .done {
    border-top-color: var(--ink-3);
  }
  .steps .current {
    border-top-color: var(--accent);
    color: var(--ink-1);
    font-weight: 600;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s-3);
  }
  .problem {
    max-width: var(--measure);
    color: var(--danger);
  }
  .link {
    justify-self: start;
    color: var(--accent);
  }
  .words {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--s-2) var(--s-4);
    padding: var(--s-4);
    border-radius: var(--r-3);
    background: var(--sunken);
    font: var(--t-mono);
    list-style: decimal inside;
    user-select: all;
  }
  .words li::marker {
    color: var(--ink-3);
  }
  @media (width < 40rem) {
    .words {
      grid-template-columns: repeat(2, 1fr);
    }
  }
</style>
