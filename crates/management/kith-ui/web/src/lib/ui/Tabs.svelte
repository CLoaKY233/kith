<!--
@component
Tabs: Bits Tabs, used by AddDevice (`Approve it live` · `Send an invite link`). A hairline
under the list; the active tab is `--ink-1` 600 with a 2 px `--ink-1` underline sitting on
that hairline, the others `--ink-2`. No coloured bar. Labels never shift when the weight
changes. Keyboard: ←/→, Home and End move and activate; Tab moves into the panel.
Props:
  - value: string, bindable.
  - tabs: readonly { value: string; label: string }[], labels from copy.ts.
  - label: string (from copy.ts), the tab list's accessible name.
  - onValueChange?: (value: string) => void.
  - panel: Snippet<[string]>, renders the panel for the given tab value. Only the active
    panel is in the page.
-->
<script lang="ts">
  import { Tabs } from 'bits-ui';
  import type { Snippet } from 'svelte';

  let {
    value = $bindable(''),
    tabs,
    label,
    onValueChange,
    panel,
  }: {
    value?: string;
    tabs: readonly { value: string; label: string }[];
    label: string;
    onValueChange?: (value: string) => void;
    panel: Snippet<[string]>;
  } = $props();
</script>

<div class="tabs">
  <Tabs.Root bind:value {onValueChange} activationMode="automatic" loop>
    <Tabs.List aria-label={label} class="kith-tab-list">
      {#each tabs as tab (tab.value)}
        <Tabs.Trigger value={tab.value} class="kith-tab">
          <span class="text" data-label={tab.label}>{tab.label}</span>
        </Tabs.Trigger>
      {/each}
    </Tabs.List>
    {#each tabs as tab (tab.value)}
      <Tabs.Content value={tab.value} class="kith-tab-panel">
        {#if tab.value === value}{@render panel(tab.value)}{/if}
      </Tabs.Content>
    {/each}
  </Tabs.Root>
</div>

<style>
  .tabs {
    display: grid;
    min-width: 0;
  }
  .tabs :global(.kith-tab-list) {
    display: flex;
    gap: var(--s-5);
    border-bottom: var(--hairline) solid var(--line);
  }
  .tabs :global(.kith-tab) {
    display: inline-grid;
    place-items: center;
    height: var(--row);
    margin-bottom: calc(-1 * var(--hairline));
    border-bottom: var(--lead-bar) solid transparent;
    color: var(--ink-2);
    font: var(--t-body);
    white-space: nowrap;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    transition:
      color var(--d-hover) var(--e-linear),
      border-color var(--d-hover) var(--e-linear);
  }
  .tabs :global(.kith-tab:hover) {
    color: var(--ink-1);
  }
  .tabs :global(.kith-tab:focus-visible) {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .tabs :global(.kith-tab[data-state='active']) {
    color: var(--ink-1);
    font-weight: 600;
    border-bottom-color: var(--ink-1);
  }
  .tabs :global(.kith-tab-panel) {
    padding-top: var(--s-5);
  }
  .tabs :global(.kith-tab-panel:focus-visible) {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: var(--focus-offset);
  }
  .text {
    display: inline-grid;
  }
  .text::after {
    content: attr(data-label);
    height: 0;
    overflow: hidden;
    visibility: hidden;
    font-weight: 600;
  }
  @media (forced-colors: active) {
    .tabs :global(.kith-tab[data-state='active']) {
      border-bottom-color: Highlight;
    }
  }
</style>
