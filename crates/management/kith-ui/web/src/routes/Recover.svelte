<!--
@component
Recover: someone with the network's 18 recovery words and no working passkey makes a new
passkey here and becomes an owner again. The words approve the change; the Console signs
nothing on its own.
-->
<script lang="ts">
  import { accounts } from '../lib/accounts';
  import { copy } from '../lib/copy';
  import { passkeyText } from '../lib/passkeyText';
  import Button from '../lib/ui/Button.svelte';
  import { createPasskey } from '../lib/webauthn';

  let name = $state('');
  let words = $state('');
  let pending = $state(false);
  let error = $state<string | null>(null);

  async function recover(): Promise<void> {
    if (pending) return;
    pending = true;
    error = null;
    try {
      const options = await accounts.recoverBegin(name);
      const made = await createPasskey(options.public_key);
      await accounts.recoverFinish(made, words.trim().split(/\s+/).join(' '));
      location.assign('/');
    } catch (cause) {
      error = passkeyText(cause);
      pending = false;
    }
  }
</script>

<section class="recover" aria-labelledby="recover-title">
  <p class="brand">{copy.brand}</p>
  <h1 id="recover-title" class="title">Recover with your words</h1>
  <label>
    Your name
    <input bind:value={name} autocomplete="name" />
  </label>
  <label>
    The 18 recovery words
    <textarea bind:value={words} rows="4" autocomplete="off" spellcheck="false"></textarea>
  </label>
  <div class="actions">
    <Button variant="primary" {pending} onclick={recover}>Make a new passkey</Button>
  </div>
  {#if error}<p class="problem" role="alert">{error}</p>{/if}
</section>

<style>
  .recover {
    display: grid;
    align-content: center;
    justify-items: stretch;
    gap: var(--s-4);
    min-height: 100dvh;
    max-width: var(--gate-max);
    margin: 0 auto;
    padding: var(--s-8) var(--gutter-mobile);
  }
  label {
    display: grid;
    gap: var(--s-2);
  }
  .brand {
    color: var(--ink-3);
  }
  .title {
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
  }
  .actions {
    display: flex;
  }
  .problem {
    color: var(--danger);
  }
</style>
