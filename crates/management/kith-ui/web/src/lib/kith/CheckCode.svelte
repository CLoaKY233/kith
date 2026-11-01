<!--
@component
CheckCode (spec §5): mono 20/600, .08em, `K7Q·4XM` on sunken, r-2. Spelled out for screen
readers ("K 7 Q, 4 X M").
Props:
  - code: string, six Crockford base32 characters (with or without the dot).
-->
<script lang="ts">
  import { copy } from '../copy';
  import { checkCode } from '../format';

  let { code }: { code: string } = $props();
</script>

<span class="code" role="img" aria-label={copy.spoken.checkCode(code)}>{checkCode(code)}</span>

<style>
  .code {
    display: inline-block;
    padding: var(--s-1) var(--s-3);
    border-radius: var(--r-2);
    background: var(--sunken);
    font: var(--t-code);
    letter-spacing: var(--t-code-tracking);
  }
</style>
