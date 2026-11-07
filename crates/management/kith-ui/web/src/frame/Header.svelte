<!--
@component
The header (spec §4.2): sticky, 56 px, `--bg`, hairline bottom. The screen title at
`--t-title`, then on the right, in order: ExitChip (only with an exit), NeedsYouButton
(only when the count is above 0), who is signed in with Sign out (a public Console only),
the connection indicator (nothing when live,
`Reconnecting…`, or `as of 14:02` once stale), and `Search ⌘K`. The connection text hangs
to the left of the cluster without taking room, so nothing moves when it appears, and it is
not a live region (frame/conn announces what matters). The bar's contents share the
content's max width, so the controls end at the content's right edge.
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
  import { me } from '../lib/account.svelte';
  import Lazy from './Lazy.svelte';
  import ExitChip from './ExitChip.svelte';
  import { layers } from './layers.svelte';
  import { apple } from './net';
  import NeedsYouButton from './NeedsYouButton.svelte';

  let { title, net }: { title: string; net: string | null } = $props();

  const connText = $derived(
    connection.shown === 'reconnecting'
      ? copy.header.reconnecting
      : connection.shown === 'stale'
        ? copy.header.staleAsOf(clock(connection.asOf))
        : '',
  );
</script>

<header class="header">
  <div class="bar">
    <h1 id="page-title" class="title">{title}</h1>
    <div class="right">
      <span class="conn">{connText}</span>
      {#if net}
        <ExitChip {net} />
        <NeedsYouButton {net} />
      {/if}
      {#if me()}<Lazy name="account">{#snippet children(Account)}<Account />{/snippet}</Lazy>{/if}
      <button
        type="button"
        class="search"
        aria-keyshortcuts={apple ? 'Meta+K' : 'Control+K'}
        onclick={() => (layers.palette = true)}
      >
        <Icon name="search" />
        <span>{copy.header.search}</span>
        <span aria-hidden="true">
          <Kbd>{apple ? copy.header.searchShortcut : copy.header.searchShortcutOther}</Kbd>
        </span>
      </button>
    </div>
  </div>
</header>

<style>
  .header {
    position: sticky;
    top: 0;
    z-index: var(--z-header);
    height: var(--header);
    background: var(--bg);
    border-bottom: var(--hairline) solid var(--line);
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
    height: 100%;
    /* Like a window's toolbar: the title over the content, the controls at the far edge. */
    padding: 0 var(--content-pad);
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
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--s-2);
    flex: none;
  }
  .conn {
    position: absolute;
    right: 100%;
    margin-right: var(--s-2);
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
