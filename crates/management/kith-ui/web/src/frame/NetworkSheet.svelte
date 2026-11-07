<!--
@component
The phone Network sheet (spec §4.4, `?sheet=network`, so Back closes it): this device in
this network, the network list with `Start or join another network`, Names, Exits, Add
device (admins), Settings, This device. MobileBars loads it the first time it opens.
Props:
  - net: string, the network id.
  - page: NavPage | null, the active page.
  - self?: boolean, true on This device.
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import Sheet from '../lib/ui/Sheet.svelte';
  import { closeOverlay, href, netHref, openOverlay, setQuery } from '../router';
  import { layers } from './layers.svelte';
  import { isAdmin, membership, networkName } from './net';
  import { SETTINGS, shaping, type NavPage } from './nav';
  import Account from './Account.svelte';
  import NavLink from './NavLink.svelte';
  import NetworkList from './NetworkList.svelte';

  let { net, page, self = false }: { net: string; page: NavPage | null; self?: boolean } = $props();

  const me = $derived(membership(net));
  /** Drops `?sheet` in place, so the link that follows leaves no sheet behind for Back. */
  const dismiss = (): void => setQuery({ sheet: null }, { replace: true });
</script>

<Sheet
  bind:open={() => layers.network, (open) => !open && closeOverlay('sheet')}
  title={networkName(net)}
  closeLabel={copy.action.close}
>
  {#if me}
    <p class="identity">
      <span class="who">{me.name}</span>
      <span class="t-mono">{me.address}</span>
      <span class="role">{copy.role[me.role]}</span>
    </p>
  {/if}
  <div class="networks">
    <NetworkList current={net} page={page ?? 'overview'} onPick={dismiss} />
  </div>
  <ul class="menu">
    {#each shaping() as item (item.page)}
      <li>
        <NavLink
          to={netHref(net, item.page)}
          icon={item.icon}
          label={item.label}
          active={page === item.page}
          onclick={dismiss}
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
            dismiss();
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
        onclick={dismiss}
      />
    </li>
    <li>
      <NavLink
        to={href({ name: 'this' })}
        icon="laptop"
        label={copy.nav.thisDevice}
        active={self}
        onclick={dismiss}
      />
    </li>
  </ul>
  <div class="account"><Account /></div>
</Sheet>

<style>
  .account {
    margin-top: var(--s-3);
  }
  .identity {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s-1) var(--s-2);
    margin-bottom: var(--s-3);
    color: var(--ink-2);
  }
  .who {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
    color: var(--ink-1);
  }
  .role {
    color: var(--ink-3);
  }
  .networks {
    margin-bottom: var(--s-3);
    padding-bottom: var(--s-3);
    border-bottom: var(--hairline) solid var(--line);
  }
  .networks :global(.item),
  .menu :global(.item) {
    min-height: var(--target-touch);
  }
  .menu {
    display: grid;
    gap: var(--s-1);
  }
</style>
