<!--
@component
Exits (spec §6.7). Placeholder; workstream E owns it.
Props:
  - net: string
-->
<script lang="ts">
  import { copy } from '../lib/copy';

  let { net }: { net: string } = $props();
</script>

<section class="screen" data-net={undefined}>
  <h1 class="t-title">{copy.nav.exits}</h1>
  <p hidden>{net}</p>
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
