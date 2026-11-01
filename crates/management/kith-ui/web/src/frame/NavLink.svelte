<!--
@component
A nav item (spec §4.2): 32 px tall (44 on touch), a 16 px icon, the label at 14/400 and an
optional count in `--ink-3`. The active one is `--ink-1` 600 on `--sunken`; no coloured bar.
In the tablet rail it is the icon alone, named by aria-label and a tooltip.
Props:
  - to: string, the href; omit it for a button (Add device).
  - icon: IconName; label: string; count?: number | null.
  - active?: boolean; rail?: boolean; strong?: boolean (ink-1 at rest, for `+ Add device`).
  - onclick?: () => void, for a button, or extra work before a link navigates.
-->
<script lang="ts">
  import Icon from '../lib/icons/Icon.svelte';
  import type { IconName } from '../lib/icons/names';
  import Tooltip from '../lib/ui/Tooltip.svelte';
  import { follow } from '../router';

  let {
    to,
    icon,
    label,
    count = null,
    active = false,
    rail = false,
    strong = false,
    onclick,
  }: {
    to?: string;
    icon: IconName;
    label: string;
    count?: number | null;
    active?: boolean;
    rail?: boolean;
    strong?: boolean;
    onclick?: () => void;
  } = $props();

  function click(event: MouseEvent): void {
    onclick?.();
    if (to) follow(event, to);
  }
</script>

{#snippet body()}
  <Icon name={icon} />
  {#if !rail}
    <span class="label">{label}</span>
    {#if count !== null}<span class="count">{count}</span>{/if}
  {/if}
{/snippet}

{#if rail}
  <Tooltip text={label} side="right">
    {#snippet trigger(props)}
      {#if to}
        <a {...props} href={to} class="item rail" class:active class:strong aria-label={label} aria-current={active ? 'page' : undefined} onclick={click}>{@render body()}</a>
      {:else}
        <button {...props} type="button" class="item rail" class:strong aria-label={label} onclick={click}>{@render body()}</button>
      {/if}
    {/snippet}
  </Tooltip>
{:else if to}
  <a href={to} class="item" class:active class:strong aria-current={active ? 'page' : undefined} onclick={click}>{@render body()}</a>
{:else}
  <button type="button" class="item" class:strong onclick={click}>{@render body()}</button>
{/if}

<style>
  .item {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: 100%;
    height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    color: var(--ink-2);
    font: var(--t-body);
    text-align: left;
    cursor: pointer;
    transition:
      background-color var(--d-hover) var(--e-linear),
      color var(--d-hover) var(--e-linear);
  }
  .item:hover {
    background: var(--sunken);
    color: var(--ink-1);
  }
  .item.strong {
    color: var(--ink-1);
  }
  .item.active {
    background: var(--sunken);
    color: var(--ink-1);
    font-weight: 600;
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font: var(--t-small);
    color: var(--ink-3);
  }
  .rail {
    justify-content: center;
    width: var(--control);
    padding: 0;
  }
  @media (pointer: coarse) {
    .item {
      height: var(--target-touch);
    }
    .rail {
      width: var(--target-touch);
    }
  }
</style>
