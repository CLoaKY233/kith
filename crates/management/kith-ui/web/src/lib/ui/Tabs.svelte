<!--
@component
Tabs: Bits Tabs, used by AddDevice (Approve it live · Send an invite link).
Props:
  - value: string, bindable.
  - tabs: { value: string; label: string }[]
  - label: string, the tab list's accessible name.
  - panel: Snippet<[string]>, renders the panel for the given tab value.
-->
<script lang="ts">
  import { Tabs } from 'bits-ui';
  import type { Snippet } from 'svelte';

  let {
    value = $bindable(''),
    tabs,
    label,
    panel,
  }: {
    value?: string;
    tabs: readonly { value: string; label: string }[];
    label: string;
    panel: Snippet<[string]>;
  } = $props();
</script>

<Tabs.Root bind:value class="kith-tabs">
  <Tabs.List aria-label={label} class="kith-tab-list">
    {#each tabs as tab (tab.value)}
      <Tabs.Trigger value={tab.value} class="kith-tab">{tab.label}</Tabs.Trigger>
    {/each}
  </Tabs.List>
  {#each tabs as tab (tab.value)}
    <Tabs.Content value={tab.value} class="kith-tab-panel">{@render panel(tab.value)}</Tabs.Content>
  {/each}
</Tabs.Root>

<style>
  :global(.kith-tabs) {
    display: grid;
    gap: var(--s-4);
  }
  :global(.kith-tab-list) {
    display: flex;
    gap: var(--s-4);
    border-bottom: var(--hairline) solid var(--line);
  }
  :global(.kith-tab) {
    height: var(--row);
    color: var(--ink-2);
    border-bottom: var(--lead-bar) solid transparent;
    cursor: pointer;
  }
  :global(.kith-tab[data-state='active']) {
    color: var(--ink-1);
    font-weight: 600;
    border-bottom-color: var(--ink-1);
  }
</style>
