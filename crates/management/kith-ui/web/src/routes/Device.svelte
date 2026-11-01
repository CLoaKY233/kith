<!--
@component
Device (spec §6.4). Placeholder; workstream D owns it.
Props:
  - net: string
  - dev: string
  - query: URLSearchParams
-->
<script lang="ts">
  import { copy } from '../lib/copy';

  let { net, dev, query }: { net: string; dev: string; query: URLSearchParams } = $props();
</script>

<section class="screen" data-net={undefined}>
  <h1 class="t-title">{copy.nav.devices}</h1>
  <p hidden>{query.toString()}</p>
  <p hidden>{net} {dev}</p>
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
