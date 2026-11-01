<!--
@component
CheckCode (spec §5, §7.2, §7.3): mono 20/600, .08em, `K7Q·4XM` on `--sunken`, `--r-2`.
Screen readers hear it spelled out ("K 7 Q, 4 X M"), never as a word.
Props:
  - code: string, six Crockford base32 characters (with or without the dot).
  - caption?: (code: string) => string. A copy.ts sentence that contains the code, such as
    `copy.checkCode.flowLabel`. The sentence is shown with the code block in its place, and
    read as one sentence with the code spelled out.
-->
<script lang="ts">
  import { copy } from '../copy';
  import { checkCode } from '../format';

  let { code, caption }: { code: string; caption?: (code: string) => string } = $props();

  /** Marks where the code sits in the caption; a private-use character never in copy. */
  const MARK = '\u{E000}';

  const shown = $derived(checkCode(code));
  const spoken = $derived(copy.spoken.checkCode(code));
  const parts = $derived.by(() => {
    if (!caption) return null;
    const [before = '', after = ''] = caption(MARK).split(MARK);
    return { before, after };
  });
</script>

{#if parts}
  <p class="caption">
    <span aria-hidden="true">{parts.before}</span><span
      class="code"
      aria-hidden="true">{shown}</span
    ><span aria-hidden="true">{parts.after}</span>
    <span class="sr-only">{parts.before}{spoken}{parts.after}</span>
  </p>
{:else}
  <span class="code" role="img" aria-label={spoken}>{shown}</span>
{/if}

<style>
  .code {
    display: inline-block;
    padding: var(--s-1) var(--s-3);
    border-radius: var(--r-2);
    background: var(--sunken);
    color: var(--ink-1);
    font: var(--t-code);
    letter-spacing: var(--t-code-tracking);
    white-space: nowrap;
  }
  /* Inline, so the sentence wraps at words around the code, never at the dash. */
  .caption {
    max-width: var(--measure);
    color: var(--ink-2);
  }
  .caption .code {
    vertical-align: middle;
  }
</style>
