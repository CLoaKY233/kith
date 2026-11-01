<!--
@component
The network switcher (spec §4.2 `▾`, §4.4 Network sheet): every network this device is in,
with its role here, then `Start or join another network`. Picking one keeps the page:
Devices in home goes to Devices in work.
Props:
  - current: string | null, the network id on screen.
  - page: NavPage, the page to keep.
  - onPick?: () => void, called after a pick (closes the popover or sheet).
-->
<script lang="ts">
  import { copy } from '../lib/copy';
  import { live } from '../lib/live/store.svelte';
  import { follow, href, netHref } from '../router';
  import { membership } from './net';
  import type { NavPage } from './nav';

  let {
    current,
    page,
    onPick,
  }: { current: string | null; page: NavPage; onPick?: () => void } = $props();

  const welcome = href({ name: 'welcome' });
</script>

<nav class="list" aria-label={copy.nav.switchNetwork}>
  <ul>
    {#each live.status?.networks ?? [] as network (network.id)}
      {@const to = netHref(network.id, page)}
      {@const role = membership(network.id)?.role}
      <li>
        <a
          href={to}
          class="item"
          aria-current={network.id === current ? 'page' : undefined}
          onclick={(e) => {
            onPick?.();
            follow(e, to);
          }}
        >
          <span class="name">{network.name}</span>
          {#if role}<span class="meta">{copy.role[role]}</span>{/if}
        </a>
      </li>
    {/each}
  </ul>
  <a
    href={welcome}
    class="item other"
    onclick={(e) => {
      onPick?.();
      follow(e, welcome);
    }}>{copy.nav.otherNetwork}</a
  >
</nav>

<style>
  .list {
    display: grid;
    gap: var(--s-1);
    min-width: calc(var(--sidebar) - 2 * var(--s-4));
  }
  ul {
    display: grid;
    gap: var(--s-1);
  }
  .item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    min-height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    color: var(--ink-2);
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .item:hover {
    background: var(--sunken);
  }
  .item[aria-current='page'] {
    background: var(--sunken);
    color: var(--ink-1);
    font-weight: 600;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .other {
    border-top: var(--hairline) solid var(--line);
    border-radius: 0 0 var(--r-2) var(--r-2);
    padding-top: var(--s-1);
  }
  @media (pointer: coarse) {
    .item {
      min-height: var(--target-touch);
    }
  }
</style>
