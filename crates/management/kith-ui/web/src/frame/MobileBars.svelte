<!--
@component
The phone frame (spec §4.4, below 640 px).
  - Top bar, 52 px: the network name as a button `home ▾` (opens the Network sheet; the
    chevron only shows with more than one network), the RoleChip, then ExitChip while an
    exit is in use, and Search on the right. The connection indicator gets its own line
    under the bar, so it never squeezes the network name.
  - Bottom tab bar, 56 px plus the safe area: Overview · Devices · Activity · Network.
    Overview carries the amber Needs-you count, named "Overview, 2 need you". Network opens
    the Network sheet (it holds no route) and is marked while on a page the sheet holds.
  - The Network sheet (`?sheet=network`, so Back closes it): this device in this network,
    the network list with `Start or join another network`, Names, Exits, Add device
    (admins), Settings, This device.
Props:
  - net: string, the network id.
  - page: NavPage | null, the active page.
  - self?: boolean, true on This device.
  - part: 'top' | 'tabs', which bar to render (the tab bar comes after the content).
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import { clock } from '../lib/format';
  import Icon from '../lib/icons/Icon.svelte';
  import { live } from '../lib/live/store.svelte';
  import IconButton from '../lib/ui/IconButton.svelte';
  import RoleChip from '../lib/ui/RoleChip.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';
  import { follow, netHref, openOverlay } from '../router';
  import { connection } from './conn.svelte';
  import ExitChip from './ExitChip.svelte';
  import { layers } from './layers.svelte';
  import { membership, networkName } from './net';
  import { OBSERVING, type NavPage } from './nav';
  import { needs } from './needs.svelte';

  let {
    net,
    page,
    self = false,
    part,
  }: { net: string; page: NavPage | null; self?: boolean; part: 'top' | 'tabs' } = $props();

  const me = $derived(membership(net));
  const count = $derived(needs.count(net));
  const many = $derived((live.status?.networks.length ?? 0) > 1);
  const inSheet = $derived(
    self || page === 'names' || page === 'exits' || page === 'settings',
  );
  const connText = $derived(
    connection.shown === 'reconnecting'
      ? copy.header.reconnecting
      : connection.shown === 'stale'
        ? copy.header.staleAsOf(clock(connection.asOf))
        : '',
  );

  const openSheet = (): void => openOverlay('sheet', 'network');

  // The Network sheet loads the first time it opens, off the first load.
  let sheetUsed = $state(false);
  $effect(() => {
    if (layers.network) sheetUsed = true;
  });

  function tab(event: MouseEvent, to: string, current: boolean): void {
    if (current && event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey) {
      event.preventDefault();
      window.scrollTo({ top: 0 });
      return;
    }
    follow(event, to);
  }
</script>

{#if part === 'top'}
  <div class="bars">
    <header class="top">
      <button
        type="button"
        class="net"
        aria-haspopup="dialog"
        aria-expanded={layers.network}
        aria-label={copy.nav.networkMenu(networkName(net))}
        onclick={openSheet}
      >
        <span>{networkName(net)}</span>
        {#if many}<Icon name="chevron-down" />{/if}
      </button>
      {#if me}
        <RoleChip
          role={me.role}
          network={networkName(net)}
          keyFpr={me.key_fpr}
          madeAdminBy={me.made_admin_by}
          madeAdminAtMs={me.made_admin_at_ms}
          admins={me.admins}
        />
      {/if}
      <ExitChip {net} compact />
      <span class="search">
        <IconButton icon="search" label={copy.header.search} onclick={() => (layers.palette = true)} />
      </span>
    </header>
    <p class="conn" class:shown={connText !== ''}>{connText}</p>
  </div>

  {#if sheetUsed}
    {#await import('./NetworkSheet.svelte') then { default: NetworkSheet }}
      <NetworkSheet {net} {page} {self} />
    {/await}
  {/if}
{:else}
  <nav class="tabs" aria-label={copy.nav.primary}>
    {#each OBSERVING as item (item.page)}
      {@const to = netHref(net, item.page)}
      {@const current = page === item.page}
      {@const badge = item.page === 'overview' && count > 0}
      <a
        href={to}
        class="tab"
        aria-current={current ? 'page' : undefined}
        onclick={(e) => tab(e, to, current)}
      >
        <span class="glyph">
          <Icon name={item.icon} />
          {#if badge}<span class="badge" aria-hidden="true"><StatusDot state="needs_you" />{count}</span>{/if}
        </span>
        <span>{item.label}</span>
        {#if badge}<span class="sr-only">{copy.nav.needsYouSuffix(count)}</span>{/if}
      </a>
    {/each}
    <button
      type="button"
      class="tab"
      class:active={inSheet}
      aria-haspopup="dialog"
      aria-expanded={layers.network}
      onclick={openSheet}
    >
      <span class="glyph"><Icon name="network" /></span>
      <span>{copy.nav.network}</span>
    </button>
  </nav>
{/if}

<style>
  .bars {
    position: sticky;
    top: 0;
    z-index: var(--z-header);
    background: var(--bg);
    border-bottom: var(--hairline) solid var(--line);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    height: var(--topbar);
    padding: 0 var(--gutter-mobile);
    overflow: hidden;
  }
  .net {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    flex: 0 1 auto;
    height: var(--target-touch);
    margin-left: calc(-1 * var(--s-2));
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
    color: var(--ink-1);
    cursor: pointer;
  }
  .net span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .search {
    margin-left: auto;
    flex: none;
  }
  /* §5 neutral connection line: takes room only while it has something to say. */
  .conn {
    display: none;
    padding: var(--s-1) var(--gutter-mobile);
    font: var(--t-small);
    color: var(--ink-2);
    background: var(--sunken);
    border-top: var(--hairline) solid var(--line);
  }
  .conn.shown {
    display: block;
  }
  .tabs {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: var(--z-header);
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    height: calc(var(--tabbar) + env(safe-area-inset-bottom));
    padding-bottom: env(safe-area-inset-bottom);
    background: var(--surface);
    border-top: var(--hairline) solid var(--line);
  }
  .tab {
    display: grid;
    place-items: center;
    align-content: center;
    gap: var(--s-1);
    min-width: 0;
    color: var(--ink-3);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    cursor: pointer;
  }
  .tab[aria-current='page'],
  .tab.active {
    color: var(--ink-1);
    font-weight: 600;
  }
  .glyph {
    position: relative;
    display: inline-grid;
  }
  .badge {
    position: absolute;
    top: calc(-1 * var(--s-1));
    left: calc(100% + var(--s-1));
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--attn-text);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
  }
</style>
