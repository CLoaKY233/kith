<!--
@component
Who is signed in to a public Console, with Sign out: in the header on wider screens and in
the Network sheet on phones. Local mode has nobody signed in and renders nothing.
-->
<script lang="ts">
  import { consoleMode, me, signOut } from '../lib/account.svelte';
  import { copy } from '../lib/copy';
  import Button from '../lib/ui/Button.svelte';

  let pending = $state(false);
  const person = $derived(consoleMode() === 'public' ? me() : null);

  function leave(): void {
    pending = true;
    void signOut();
  }
</script>

{#if person}
  <div class="account">
    <p class="who">
      <span class="name">{person.name}</span>
      <span class="role">{copy.account.role[person.role]}</span>
    </p>
    <Button variant="ghost" {pending} onclick={leave}>{copy.account.signOut}</Button>
  </div>
{/if}

<style>
  .account {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .who {
    display: inline-flex;
    align-items: baseline;
    gap: var(--s-2);
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .role {
    font: var(--t-small);
    color: var(--ink-3);
  }
</style>
