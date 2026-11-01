<!--
@component
The header (spec §4.2): sticky, 56 px, `--bg`, hairline bottom. The screen title at
`--t-title`, then on the right, in order: ExitChip (only with an exit), NeedsYouButton
(only when the count is above 0), the connection indicator (nothing when live,
`Reconnecting…`, or `as of 14:02` once stale), and `Search ⌘K`.
Props:
  - title: string, the screen title (from copy.ts).
  - net: string | null, the network id; null hides the network parts.
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import { clock } from '../lib/format';
  import Icon from '../lib/icons/Icon.svelte';
  import Kbd from '../lib/ui/Kbd.svelte';
  import { connection } from './conn.svelte';
  import ExitChip from './ExitChip.svelte';
  import { layers } from './layers.svelte';
  import { apple } from './net';
  import NeedsYouButton from './NeedsYouButton.svelte';

  let { title, net }: { title: string; net: string | null } = $props();
</script>

<header class="header">
  <h1 class="title">{title}</h1>
  <div class="right">
    {#if net}
      <ExitChip {net} />
      <NeedsYouButton {net} />
    {/if}
    {#if connection.shown === 'reconnecting'}
      <span class="conn" role="status">{copy.header.reconnecting}</span>
    {:else if connection.shown === 'stale'}
      <span class="conn" role="status">{copy.header.staleAsOf(clock(connection.asOf))}</span>
    {/if}
    <button type="button" class="search" onclick={() => (layers.palette = true)}>
      <Icon name="search" />
      <span>{copy.header.search}</span>
      <Kbd>{apple ? copy.header.searchShortcut : copy.header.searchShortcutOther}</Kbd>
    </button>
  </div>
</header>

<style>
  .header {
    position: sticky;
    top: 0;
    z-index: var(--z-header);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
    height: var(--header);
    padding: 0 var(--content-pad);
    background: var(--bg);
    border-bottom: var(--hairline) solid var(--line);
  }
  .title {
    font: var(--t-title);
    letter-spacing: var(--t-title-tracking);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .right {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    flex: none;
  }
  .conn {
    font: var(--t-small);
    color: var(--ink-3);
    white-space: nowrap;
  }
  .search {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    height: var(--control);
    padding: 0 var(--s-2) 0 var(--s-3);
    border-radius: var(--r-2);
    border: var(--hairline) solid var(--line);
    background: var(--surface);
    color: var(--ink-2);
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .search:hover {
    background: var(--sunken);
    color: var(--ink-1);
  }
</style>
