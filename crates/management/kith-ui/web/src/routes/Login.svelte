<!--
@component
Login (docs/plans/console.md, flow 2): the signed-out page of a public Console, and `/login`
in either mode. One button, `Sign in with passkey`, and nothing about the network. The
server's challenge is fetched on arrival so the prompt opens straight from the click
(Safari wants a user gesture). On success the server serves the signed-in page.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { accounts } from '../lib/accounts';
  import { ApiError } from '../lib/api';
  import { copy } from '../lib/copy';
  import { copyConsole } from '../lib/copy.console';
  import { passkeyText } from '../lib/passkeyText';
  import Button from '../lib/ui/Button.svelte';
  import { passkeysSupported, usePasskey, type RequestOptionsJSON } from '../lib/webauthn';

  let pending = $state(false);
  let error = $state<string | null>(passkeysSupported() ? null : copyConsole.passkey.unsupported);
  let options: Promise<RequestOptionsJSON> | null = null;

  function fetchOptions(): void {
    const next = accounts.loginBegin().then((answer) => answer.public_key);
    next.catch(() => undefined);
    options = next;
  }

  onMount(fetchOptions);

  async function signIn(): Promise<void> {
    if (pending) return;
    if (!passkeysSupported()) {
      error = copyConsole.passkey.unsupported;
      return;
    }
    pending = true;
    error = null;
    try {
      if (!options) fetchOptions();
      const assertion = await usePasskey(await (options as Promise<RequestOptionsJSON>));
      await accounts.loginFinish(assertion);
      // The server now serves the signed-in page; `/login` itself has nothing to show.
      if (location.pathname === '/login') location.assign('/');
      else location.reload();
    } catch (cause) {
      const unknown = cause instanceof ApiError && (cause.code === 'unauthorized' || cause.code === 'not_found');
      error = unknown ? copyConsole.login.unknown : passkeyText(cause);
      pending = false;
      fetchOptions();
    }
  }
</script>

<section class="login" aria-labelledby="login-title">
  <p class="brand">{copy.brand}</p>
  <h1 id="login-title" class="title">{copyConsole.login.title}</h1>
  <Button variant="primary" {pending} pendingLabel={copyConsole.login.signingIn} onclick={signIn}>
    {copyConsole.login.signIn}
  </Button>
  {#if error}<p class="problem" role="alert">{error}</p>{/if}
</section>

<style>
  .login {
    display: grid;
    align-content: center;
    justify-items: start;
    gap: var(--s-4);
    min-height: 100dvh;
    max-width: var(--gate-max);
    margin: 0 auto;
    padding: var(--s-8) var(--gutter-mobile);
  }
  .brand {
    color: var(--ink-3);
  }
  .title {
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
  }
  .problem {
    max-width: var(--measure);
    color: var(--danger);
  }
</style>
