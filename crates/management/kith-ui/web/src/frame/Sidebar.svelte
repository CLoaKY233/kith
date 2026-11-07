<!--
@component
The sidebar (spec §4.2, ≥1024 px) and the tablet rail (§4.3, 640–1023 px). `--surface`
with a 1 px right hairline. IdentityBlock, then Observing (Overview, Devices, Activity),
24 px, Shaping (Names, Exits), 24 px, `+ Add device` (admins), and Settings pinned to the
bottom. The rail is 64 px of icons with tooltips; IdentityBlock becomes a 32 px initial.
Props:
  - net: string, the network id the nav points into.
  - page: NavPage | null, the active page.
  - rail?: boolean (default false).
  - self?: boolean, true on This device (the identity block is the active item).
-->
<script lang="ts">
  import { me as signedIn } from '../lib/account.svelte';
  import IdentityBlock from '../lib/kith/IdentityBlock.svelte';
  import { copy } from '../lib/copy';
  import { netHref, openOverlay } from '../router';
  import { isAdmin, peers } from './net';
  import { OBSERVING, SETTINGS, shaping, type NavItem, type NavPage } from './nav';
  import NavLink from './NavLink.svelte';
  import NetworkList from './NetworkList.svelte';

  let {
    net,
    page,
    rail = false,
    self = false,
  }: { net: string; page: NavPage | null; rail?: boolean; self?: boolean } = $props();

  const devices = $derived(peers(net).length + 1);
</script>

{#snippet link(item: NavItem)}
  <li>
    <NavLink
      to={netHref(net, item.page)}
      icon={item.icon}
      label={item.label}
      count={item.page === 'devices' ? devices : null}
      active={page === item.page}
      {rail}
    />
  </li>
{/snippet}

<aside class="side" class:rail>
  <div class="identity">
    <IdentityBlock {net} compact={rail} current={self}>
      {#snippet switcher(close)}
        <NetworkList current={net} page={page ?? 'overview'} onPick={close} />
      {/snippet}
    </IdentityBlock>
  </div>
  <nav class="nav" aria-label={copy.nav.primary}>
    <ul class="group">
      {#each OBSERVING as item (item.page)}{@render link(item)}{/each}
    </ul>
    <ul class="group">
      {#each shaping() as item (item.page)}{@render link(item)}{/each}
    </ul>
    {#if isAdmin(net) || signedIn() !== null}
      <ul class="group">
        <li>
          <NavLink
            icon="plus"
            label={copy.nav.addDevice}
            strong
            {rail}
            onclick={() => openOverlay('add', 'device')}
          />
        </li>
      </ul>
    {/if}
    <ul class="group bottom">
      {@render link(SETTINGS)}
    </ul>
  </nav>
</aside>

<style>
  .side {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    width: var(--sidebar);
    height: 100dvh;
    padding: var(--s-5) var(--s-4) var(--s-4);
    background: var(--surface);
    border-right: var(--hairline) solid var(--line);
    overflow-y: auto;
  }
  .side.rail {
    width: var(--rail);
    align-items: center;
    padding-inline: var(--s-2);
  }
  .identity {
    padding-inline: var(--s-2);
  }
  .rail .identity {
    padding-inline: 0;
  }
  .nav {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--s-5);
    min-height: 0;
  }
  .group {
    display: grid;
    gap: var(--s-1);
  }
  .rail .group {
    justify-items: center;
  }
  .bottom {
    margin-top: auto;
    padding-top: var(--s-4);
    border-top: var(--hairline) solid var(--line);
  }
</style>
