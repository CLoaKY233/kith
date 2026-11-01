<!--
@component
The shortcut sheet (spec §10), opened with `?`. Lists every global shortcut.
Props: none (open state is `layers.shortcuts`).
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Kbd from '../lib/ui/Kbd.svelte';
  import { layers } from './layers.svelte';
  import { apple } from './net';

  const k = copy.shortcuts.keys;
  const rows: readonly (readonly [string, string])[] = [
    [apple ? k.palette : k.paletteOther, copy.shortcuts.palette],
    [k.filter, copy.shortcuts.filter],
    [k.help, copy.shortcuts.help],
    [k.goOverview, copy.shortcuts.goOverview],
    [k.goDevices, copy.shortcuts.goDevices],
    [k.goActivity, copy.shortcuts.goActivity],
    [k.close, copy.shortcuts.close],
    [k.approveDeny, copy.shortcuts.approveDeny],
  ];
</script>

<Dialog bind:open={layers.shortcuts} title={copy.shortcuts.title}>
  <dl class="keys">
    {#each rows as [key, what] (key)}
      <div class="row">
        <dt>{what}</dt>
        <dd><Kbd>{key}</Kbd></dd>
      </div>
    {/each}
  </dl>
</Dialog>

<style>
  .keys {
    display: grid;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
    min-height: var(--control);
    border-bottom: var(--hairline) solid var(--line);
  }
  .row:last-child {
    border-bottom: none;
  }
  dt {
    color: var(--ink-2);
  }
</style>
