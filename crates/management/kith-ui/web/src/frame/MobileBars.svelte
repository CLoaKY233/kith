<!--
@component
The phone frame (spec §4.4, below 640 px).
  - Top bar, 52 px: the network name as a button `home ▾` (opens the Network sheet), the
    RoleChip, then ExitChip while an exit is in use; the connection indicator on the right.
  - Bottom tab bar, 56 px plus the safe area: Overview · Devices · Activity · Network.
    Overview carries the amber Needs-you count, announced as "Overview, 2 need you".
    Network opens the Network sheet (it holds no route).
  - The Network sheet: switch network, Names, Exits, Add device (admins), Settings,
    This device.
Props:
  - net: string, the network id.
  - page: NavPage | null, the active page.
  - part: 'top' | 'tabs', which bar to render (the tab bar comes after the content).
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import { clock } from '../lib/format';
  import Icon from '../lib/icons/Icon.svelte';
  import { live } from '../lib/live/store.svelte';
  import RoleChip from '../lib/ui/RoleChip.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import StatusDot from '../lib/ui/StatusDot.svelte';
  import { follow, href, netHref, openOverlay } from '../router';
  import { connection } from './conn.svelte';
  import ExitChip from './ExitChip.svelte';
  import { layers } from './layers.svelte';
  import { isAdmin, membership, networkName } from './net';
  import { OBSERVING, SETTINGS, SHAPING, type NavPage } from './nav';
  import NavLink from './NavLink.svelte';
  import { needs } from './needs.svelte';
  import NetworkList from './NetworkList.svelte';

  let { net, page, part }: { net: string; page: NavPage | null; part: 'top' | 'tabs' } =
    $props();

  const me = $derived(membership(net));
  const count = $derived(needs.count(net));
  const many = $derived((live.status?.networks.length ?? 0) > 1);
  const close = (): void => {
    layers.network = false;
  };
</script>

{#if part === 'top'}
  <header class="top">
    <button type="button" class="net" onclick={() => (layers.network = true)}>
      <span>{networkName(net)}</span>
      <Icon name="chevron-down" />
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
    <ExitChip {net} />
    {#if connection.shown === 'reconnecting'}
      <span class="conn" role="status">{copy.header.reconnecting}</span>
    {:else if connection.shown === 'stale'}
      <span class="conn" role="status">{copy.header.staleAsOf(clock(connection.asOf))}</span>
    {/if}
  </header>

  <Sheet bind:open={layers.network} title={copy.nav.network} closeLabel={copy.action.close}>
    {#if many}
      <NetworkList current={net} page={page ?? 'overview'} onPick={close} />
    {/if}
    <ul class="menu">
      {#each SHAPING as item (item.page)}
        <li>
          <NavLink
            to={netHref(net, item.page)}
            icon={item.icon}
            label={item.label}
            active={page === item.page}
            onclick={close}
          />
        </li>
      {/each}
      {#if isAdmin(net)}
        <li>
          <NavLink
            icon="plus"
            label={copy.nav.addDevice}
            strong
            onclick={() => {
              close();
              openOverlay('add', 'device');
            }}
          />
        </li>
      {/if}
      <li>
        <NavLink
          to={netHref(net, SETTINGS.page)}
          icon={SETTINGS.icon}
          label={SETTINGS.label}
          active={page === SETTINGS.page}
          onclick={close}
        />
      </li>
      <li>
        <NavLink to={href({ name: 'this' })} icon="laptop" label={copy.nav.thisDevice} onclick={close} />
      </li>
    </ul>
  </Sheet>
{:else}
  <nav class="tabs" aria-label={copy.nav.primary}>
    {#each OBSERVING as item (item.page)}
      {@const to = netHref(net, item.page)}
      {@const badge = item.page === 'overview' && count > 0}
      <a
        href={to}
        class="tab"
        aria-current={page === item.page ? 'page' : undefined}
        aria-label={badge ? copy.nav.overviewNeedsYou(count) : undefined}
        onclick={(e) => follow(e, to)}
      >
        <span class="glyph">
          <Icon name={item.icon} />
          {#if badge}<span class="badge" aria-hidden="true"><StatusDot state="needs_you" />{count}</span>{/if}
        </span>
        <span>{item.label}</span>
      </a>
    {/each}
    <button
      type="button"
      class="tab"
      aria-haspopup="dialog"
      aria-expanded={layers.network}
      onclick={() => (layers.network = true)}
    >
      <span class="glyph"><Icon name="network" /></span>
      <span>{copy.nav.network}</span>
    </button>
  </nav>
{/if}

<style>
  .top {
    position: sticky;
    top: 0;
    z-index: var(--z-header);
    display: flex;
    align-items: center;
    gap: var(--s-2);
    height: var(--topbar);
    padding: 0 var(--gutter-mobile);
    background: var(--bg);
    border-bottom: var(--hairline) solid var(--line);
    overflow: hidden;
  }
  .net {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
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
  .conn {
    margin-left: auto;
    font: var(--t-small);
    color: var(--ink-3);
    white-space: nowrap;
  }
  .menu {
    display: grid;
    gap: var(--s-1);
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
  .tab[aria-current='page'] {
    color: var(--ink-1);
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
    font-weight: 600;
  }
</style>
