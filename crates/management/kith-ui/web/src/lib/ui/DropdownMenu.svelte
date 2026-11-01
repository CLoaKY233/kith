<!--
@component
DropdownMenu (spec §5): the ⋯ menu on rows and the sort menu. Bits DropdownMenu with
`--shadow-pop`. Members never see disabled items: screens pass the "Ask an admin" item
instead.
Props:
  - trigger: Snippet<[Record<string, unknown>]>. Spread the props on the trigger (an IconButton's `triggerProps`).
  - items: MenuItem[], where MenuItem = { label: string; onSelect: () => void; tone?: 'danger' }
    or { separator: true }.
  - open?: boolean, bindable.
-->
<script lang="ts" module>
  export type MenuItem =
    | { label: string; onSelect: () => void; tone?: 'danger' }
    | { separator: true };
</script>

<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import type { Snippet } from 'svelte';

  let {
    trigger,
    items,
    open = $bindable(false),
  }: {
    trigger: Snippet<[Record<string, unknown>]>;
    items: readonly MenuItem[];
    open?: boolean;
  } = $props();
</script>

<DropdownMenu.Root bind:open>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      {@render trigger(props)}
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    <DropdownMenu.Content class="kith-pop kith-menu" align="end" sideOffset={4}>
      {#each items as item, index (index)}
        {#if 'separator' in item}
          <DropdownMenu.Separator class="kith-menu-sep" />
        {:else}
          <DropdownMenu.Item class="kith-menu-item {item.tone ?? ''}" onSelect={item.onSelect}>
            {item.label}
          </DropdownMenu.Item>
        {/if}
      {/each}
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>

<style>
  :global(.kith-menu) {
    padding: var(--s-1);
    min-width: calc(var(--sidebar) - var(--s-7));
  }
  :global(.kith-menu-item) {
    display: flex;
    align-items: center;
    height: var(--control);
    padding: 0 var(--s-2);
    border-radius: var(--r-1);
    cursor: pointer;
    outline: none;
  }
  :global(.kith-menu-item[data-highlighted]) {
    background: var(--sunken);
  }
  :global(.kith-menu-item.danger) {
    color: var(--danger);
  }
  :global(.kith-menu-sep) {
    height: var(--hairline);
    margin: var(--s-1) 0;
    background: var(--line);
  }
</style>
