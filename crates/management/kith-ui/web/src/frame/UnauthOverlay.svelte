<!--
@component
The 401 overlay (spec §6.9, §8.5 unauth): after kith-ui restarts under an open tab, the
last view stays, dimmed, under `kith-ui restarted. Run kith ui to continue.` with the
command to copy. Nothing behind it can be used.
Props: none.
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import CopyField from '../lib/ui/CopyField.svelte';
</script>

<div class="scrim">
  <div class="card" role="alertdialog" aria-modal="true" aria-labelledby="unauth-title">
    <h2 id="unauth-title" class="title">{copy.conn.unauth}</h2>
    <CopyField label={copy.gate.commandLabel} value={copy.gate.authCommand} />
    <p class="meta">{copy.gate.authOnce}</p>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: var(--z-dialog);
    display: grid;
    place-items: center;
    padding: var(--gutter-mobile);
    background: var(--scrim);
  }
  .card {
    display: grid;
    gap: var(--s-4);
    width: min(var(--dialog), 100%);
    padding: var(--s-5);
    border-radius: var(--r-3);
    background: var(--surface);
    box-shadow: var(--shadow-modal);
  }
  .title {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
  }
  .meta {
    color: var(--ink-3);
  }
</style>
