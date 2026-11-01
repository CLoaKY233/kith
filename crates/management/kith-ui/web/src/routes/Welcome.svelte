<!--
@component
Welcome (spec §6.1). Placeholder; workstream D owns it.
Props:
  none
-->
<script lang="ts">
  import { copy } from '../lib/copy';
</script>

<section class="screen" data-net={undefined}>
  <h1 class="t-title">{copy.welcome.title}</h1>
  <p class="lede">{copy.welcome.lede}</p>
  
</section>

<style>
  .screen {
    display: grid;
    gap: var(--s-5);
    max-width: var(--content-max);
    margin: 0 auto;
    padding: var(--s-6) var(--content-pad);
  }
  @media (max-width: 40rem) {
    .screen {
      padding: var(--s-4) var(--gutter-mobile);
    }
  }
</style>
