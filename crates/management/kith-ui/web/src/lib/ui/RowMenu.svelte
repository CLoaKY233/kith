<!--
@component
RowMenu: a DropdownMenu that loads after the first paint, for the ⋯ menus on Overview's
rows (spec §1.4). Until it has loaded, the trigger renders on its own; an `open` set
meanwhile opens the menu once it is there. Same props as DropdownMenu.
-->
<script lang="ts" module>
  import type { MenuItem as Item } from './DropdownMenu.svelte';

  export type MenuItem = Item;
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    trigger,
    items,
    open = $bindable(false),
    label,
    align = 'end',
    side = 'bottom',
    onCloseAutoFocus,
  }: {
    trigger: Snippet<[Record<string, unknown>]>;
    items: readonly MenuItem[];
    open?: boolean;
    label?: string;
    align?: 'start' | 'center' | 'end';
    side?: 'top' | 'bottom';
    onCloseAutoFocus?: (event: Event) => void;
  } = $props();
</script>

{#await import('./DropdownMenu.svelte')}
  {@render trigger({ 'aria-haspopup': 'menu', 'aria-expanded': false })}
{:then { default: DropdownMenu }}
  <DropdownMenu bind:open {trigger} {items} {label} {align} {side} {onCloseAutoFocus} />
{/await}
