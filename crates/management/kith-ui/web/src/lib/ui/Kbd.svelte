<!--
@component
Kbd: a key or shortcut, such as `⌘K`.
Props:
  - children: the key text (from copy.ts).
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
</script>

<kbd class="kbd">{@render children()}</kbd>

<style>
  .kbd {
    display: inline-flex;
    align-items: center;
    padding: 0 var(--s-1);
    border-radius: var(--r-1);
    border: var(--hairline) solid var(--line);
    background: var(--sunken);
    color: var(--ink-3);
    font: var(--t-micro);
  }
</style>
