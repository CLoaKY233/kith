<!--
@component
IdentityBlock (spec §4.2, §4.3): who this device is in the network on screen.
  - Full (sidebar): the device name at 15/600 (a link to This device), then the mono
    address · network name, with `▾` only when more than one network exists (it opens the
    network switcher), then the RoleChip.
  - Compact (tablet rail): a 32 px circle with the device's initial; it opens a popover
    holding the full block and the switcher.
The RoleChip reads only its props, so it changes before the content when the network does.
Props:
  - net: string | null, the network id (null on This device).
  - compact?: boolean (default false).
  - switcher?: Snippet<[() => void]>, the network list; it gets a function that closes the
    popover it sits in.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { membership, networkName } from '../../frame/net';
  import { follow, href, subscribe } from '../../router';
  import { copy, DOT } from '../copy';
  import Icon from '../icons/Icon.svelte';
  import { live } from '../live/store.svelte';
  import Popover from '../ui/Popover.svelte';
  import RoleChip from '../ui/RoleChip.svelte';

  let {
    net,
    compact = false,
    switcher,
  }: { net: string | null; compact?: boolean; switcher?: Snippet<[() => void]> } = $props();

  const me = $derived(net ? membership(net) : null);
  const name = $derived(me?.name ?? live.status?.host.name ?? '');
  const many = $derived((live.status?.networks.length ?? 0) > 1);
  const thisHref = href({ name: 'this' });

  let onThis = $state(false);
  $effect(() =>
    subscribe((route) => {
      onThis = route.name === 'this';
    }),
  );

  let switchOpen = $state(false);
  let compactOpen = $state(false);
</script>

{#snippet block(close: () => void)}
  <div class="block">
    <a
      class="name"
      href={thisHref}
      aria-current={onThis ? 'page' : undefined}
      onclick={(e) => { close(); follow(e, thisHref); }}>{name}</a>
    {#if me && net}
      {#if many && switcher && !compact}
        <Popover bind:open={switchOpen} label={copy.nav.switchNetwork}>
          {#snippet trigger(props)}
            <button type="button" class="where switch" {...props}>
              <span class="addr">{me.address}</span><span>{DOT.trim()}</span><span>{networkName(net)}</span>
              <Icon name="chevron-down" />
            </button>
          {/snippet}
          {@render switcher(() => {
            switchOpen = false;
            close();
          })}
        </Popover>
      {:else}
        <p class="where">
          <span class="addr">{me.address}</span><span>{DOT.trim()}</span><span>{networkName(net)}</span>
        </p>
      {/if}
      <div>
        <RoleChip
          role={me.role}
          network={networkName(net)}
          keyFpr={me.key_fpr}
          madeAdminBy={me.made_admin_by}
          madeAdminAtMs={me.made_admin_at_ms}
          admins={me.admins}
        />
      </div>
    {/if}
  </div>
{/snippet}

{#if compact}
  <Popover bind:open={compactOpen} side="right" label={copy.nav.identity(name)}>
    {#snippet trigger(props)}
      <button type="button" class="initial" aria-label={copy.nav.identity(name)} {...props}>
        {name.slice(0, 1).toUpperCase()}
      </button>
    {/snippet}
    <div class="compact">
      {@render block(() => (compactOpen = false))}
      {#if many && switcher && net}
        {@render switcher(() => (compactOpen = false))}
      {/if}
    </div>
  </Popover>
{:else}
  {@render block(() => undefined)}
{/if}

<style>
  .block {
    display: grid;
    gap: var(--s-1);
    justify-items: start;
    min-width: 0;
  }
  .name {
    font: var(--t-heading);
    letter-spacing: var(--t-heading-tracking);
    color: var(--ink-1);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border-radius: var(--r-1);
  }
  .name:hover,
  .name[aria-current='page'] {
    text-decoration: underline;
    text-decoration-thickness: var(--hairline);
    text-underline-offset: var(--s-1);
  }
  .where {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    max-width: 100%;
    font: var(--t-small);
    color: var(--ink-3);
    white-space: nowrap;
  }
  .where > span:last-of-type {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .switch {
    margin-inline: calc(-1 * var(--s-1));
    padding: 0 var(--s-1);
    border-radius: var(--r-1);
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .switch:hover,
  .switch[aria-expanded='true'] {
    background: var(--sunken);
    color: var(--ink-1);
  }
  .addr {
    font-family: var(--ff-mono);
  }
  .block > div {
    padding-top: var(--s-1);
  }
  .initial {
    display: grid;
    place-items: center;
    width: var(--control);
    height: var(--control);
    border-radius: var(--r-full);
    background: var(--sunken);
    color: var(--ink-1);
    font: var(--t-heading);
    cursor: pointer;
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .initial:hover,
  .initial[aria-expanded='true'] {
    background: var(--line);
  }
  .compact {
    display: grid;
    gap: var(--s-4);
    min-width: calc(var(--sidebar) - 2 * var(--s-4));
  }
</style>
