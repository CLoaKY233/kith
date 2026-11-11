<!--
@component
Register (docs/plans/console.md, flow 3): the one-time `/register/<token>` link an owner or
admin made for a new person. No app frame, centred, max 520 px. It says who invited them,
to which network and as what, then makes their passkey. After that their inviter approves
them with a passkey of their own, and they sign in. Every failure says why in words: a
lapsed or used link, an admin's link not yet confirmed, a closed prompt.
Props:
  - token: string, from the URL.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { accounts } from '../lib/accounts';
  import { ApiError } from '../lib/api';
  import { copy } from '../lib/copy';
  import { copyConsole } from '../lib/copy.console';
  import { passkeyText } from '../lib/passkeyText';
  import type { RegisterInfo } from '../lib/types.gen';
  import Button from '../lib/ui/Button.svelte';
  import { createPasskey, passkeysSupported, type CreationOptionsJSON } from '../lib/webauthn';

  let { token }: { token: string } = $props();

  let info = $state.raw<RegisterInfo | null>(null);
  let phase = $state<'loading' | 'ready' | 'creating' | 'registered' | 'blocked'>('loading');
  let problem = $state<string | null>(null);
  let options: Promise<CreationOptionsJSON> | null = null;

  /** The prompt is fetched ahead, so it opens straight from the click. */
  function fetchOptions(): void {
    const next = accounts.registerBegin(token).then((answer) => answer.public_key);
    next.catch(() => undefined);
    options = next;
  }

  /** Why the link or the prompt failed, and whether trying again can help. */
  function explain(cause: unknown): { text: string; final: boolean } {
    if (cause instanceof ApiError) {
      switch (cause.code) {
        case 'session_expired':
          return { text: copyConsole.register.expired, final: true };
        case 'session_used':
          return { text: copyConsole.register.used, final: true };
        case 'not_found':
          return { text: copyConsole.register.invalid, final: true };
        case 'permission_denied':
          return { text: copyConsole.register.unconfirmed(info?.invited_by ?? ''), final: false };
      }
    }
    return { text: passkeyText(cause), final: false };
  }

  onMount(() => {
    accounts.registerInfo(token).then(
      (answer) => {
        info = answer;
        if (answer.registered) {
          phase = 'registered';
        } else if (!passkeysSupported()) {
          problem = copyConsole.passkey.unsupported;
          phase = 'blocked';
        } else {
          phase = 'ready';
          fetchOptions();
        }
      },
      (cause: unknown) => {
        problem = explain(cause).text;
        phase = 'blocked';
      },
    );
  });

  async function create(): Promise<void> {
    if (phase !== 'ready') return;
    phase = 'creating';
    problem = null;
    try {
      if (!options) fetchOptions();
      const credential = await createPasskey(await (options as Promise<CreationOptionsJSON>));
      await accounts.registerFinish(token, credential);
      phase = 'registered';
    } catch (cause) {
      const why = explain(cause);
      problem = why.text;
      phase = why.final ? 'blocked' : 'ready';
      if (!why.final) fetchOptions();
    }
  }
</script>

<section class="register" aria-labelledby="register-title">
  <header class="top">
    <p class="brand">{copy.brand}</p>
    <h1 id="register-title" class="title">
      {info ? copyConsole.register.title(info.network.name) : copyConsole.register.titlePlain}
    </h1>
  </header>

  {#if phase === 'loading'}
    <p class="lede" role="status">{copyConsole.register.checking}</p>
  {:else if phase === 'registered' && info}
    <div class="body">
      <h2 class="heading">{copyConsole.register.madeHeading}</h2>
      <p class="lede" role="status">{copyConsole.register.made(info.invited_by)}</p>
      <a class="link" href="/login">{copyConsole.register.signIn}</a>
    </div>
  {:else if phase === 'blocked'}
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
  {:else if info}
    <div class="body">
      <p class="lede">{copyConsole.register.lede(info.name, info.invited_by, info.role)}</p>
      <p class="lede">{copyConsole.register.passkey}</p>
      {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
      <div class="actions">
        <Button
          variant="primary"
          pending={phase === 'creating'}
          pendingLabel={copyConsole.setup.creating}
          onclick={create}
        >
          {copyConsole.setup.createPasskey}
        </Button>
      </div>
    </div>
  {/if}
</section>

<style>
  .register {
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
  }
  .lede {
    max-width: var(--measure);
    color: var(--ink-2);
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
</style>
