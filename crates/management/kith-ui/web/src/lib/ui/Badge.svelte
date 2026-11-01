<!--
@component
Badge: a short text tag, such as `you`, `flapping` or a device tag. Text only, never an
icon alone, `--r-1`, 20 px tall, one line. It has a fixed height, so a badge appearing
never moves the line it sits on.
Props:
  - tone?: 'neutral' | 'quiet' | 'attn' | 'danger' (default 'neutral').
    neutral: `--ink-2` on `--sunken` · quiet: `--ink-3`, no fill (`flapping`; only on
    `--bg` or `--surface`) · attn: `--attn-text` on `--attn-wash` · danger: `--danger`
    on `--danger-wash`.
  - children: the text, from copy.ts.
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
    flex: none;
    align-items: center;
    height: var(--chip);
    padding: 0 var(--s-1);
    /* Transparent, so forced colours draw it. */
    border: var(--hairline) solid transparent;
    border-radius: var(--r-1);
    font: var(--t-small);
    white-space: nowrap;
    vertical-align: middle;
    background: var(--sunken);
    color: var(--ink-2);
  }
  .quiet {
    padding: 0;
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
