<!--
@component
Badge: a short text tag (`you`, `flapping`, device tags). Text only, `--r-1`. Appearing
never moves layout.
Props:
  - tone?: 'neutral' | 'quiet' | 'attn' | 'danger' (default 'neutral')
  - children: the text.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    tone = 'neutral',
    children,
  }: { tone?: 'neutral' | 'quiet' | 'attn' | 'danger'; children: Snippet } = $props();
</script>

<span class="badge {tone}">{@render children()}</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    height: var(--chip);
    padding: 0 var(--s-1);
    border-radius: var(--r-1);
    font: var(--t-small);
    background: var(--sunken);
    color: var(--ink-2);
  }
  .quiet {
    background: transparent;
    color: var(--ink-3);
  }
  .attn {
    background: var(--attn-wash);
    color: var(--attn-text);
  }
  .danger {
    background: var(--danger-wash);
    color: var(--danger);
  }
</style>
