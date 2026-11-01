<!--
@component
The 401 overlay (spec §6.9, §8.5 unauth): after kith-ui restarts under an open tab, the
last view stays, dimmed, under `kith-ui restarted. Run kith ui to continue.` with the
command to copy. Nothing behind it can be used: App.svelte makes the page inert, closes the
frame's layers and ignores shortcuts while this shows. Focus starts on Copy. It fades in
like a dialog (§3.5); in dark the scrim is deeper so the dimming shows.
Props: none.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, scale } from 'svelte/transition';
  import { copy } from '../lib/copy';
  import { MOTION } from '../lib/live/timing';
  import CopyField from '../lib/ui/CopyField.svelte';

  const reduced =
    typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const duration = reduced ? 0 : MOTION.dialog;

  let card = $state<HTMLElement | null>(null);
  onMount(() => {
    (card?.querySelector<HTMLElement>('button') ?? card)?.focus();
  });
</script>

<div class="scrim" transition:fade={{ duration }}>
  <div
    class="card"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="unauth-title"
    tabindex="-1"
    bind:this={card}
    transition:scale={{ duration, start: 0.97, opacity: 0 }}
  >
    <h2 id="unauth-title" class="title">{copy.conn.unauth}</h2>
    <CopyField value={copy.gate.authCommand} />
    <p class="meta">{copy.gate.authOnce}</p>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: calc(var(--z-dialog) + 1);
    display: grid;
    place-items: center;
    padding: var(--gutter-mobile);
    background: var(--scrim);
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme='light'])) .scrim {
      background: rgb(0 0 0 / 0.6);
    }
  }
  :global(:root[data-theme='dark']) .scrim {
    background: rgb(0 0 0 / 0.6);
  }
  .card {
    display: grid;
    gap: var(--s-4);
    width: min(var(--dialog), 100%);
    padding: var(--s-5);
    border-radius: var(--r-3);
    background: var(--surface);
    box-shadow: var(--shadow-modal);
    outline: none;
  }
  .title {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
  }
  .meta {
    color: var(--ink-3);
  }
</style>
