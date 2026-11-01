<!--
@component
Kbd: a key or shortcut, such as `⌘K` in the header's `Search ⌘K` and in the shortcut sheet.
`--ink-2` on `--sunken` (the same pair as body text on an input), hairline border,
`--r-1`, at least as wide as it is tall.
Props:
  - children: the key text (from copy.ts, `header.searchShortcut` and similar).
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
</script>

<kbd class="kbd">{@render children()}</kbd>

<style>
  .kbd {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    min-width: var(--chip);
    height: var(--chip);
    padding: 0 var(--s-1);
    border: var(--hairline) solid var(--line);
    border-radius: var(--r-1);
    background: var(--sunken);
    color: var(--ink-2);
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    white-space: nowrap;
  }
</style>
