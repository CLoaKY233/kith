<!--
@component
Map (spec §9.2): hand-built SVG ring layout. Placeholder; workstream F owns it.
Props:
  - net: string, network id.
  - focused: string | null, the focused device id.
  - onSelect: (node: string) => void.
-->
<script lang="ts">
  let {
    net,
    focused,
    onSelect,
  }: { net: string; focused: string | null; onSelect: (node: string) => void } = $props();
</script>

<svg class="map" data-net={net} role="group">
  {#if focused}
    <g
      role="button"
      tabindex="0"
      aria-label={focused}
      onclick={() => onSelect(focused)}
      onkeydown={(event) => event.key === 'Enter' && onSelect(focused)}
    ></g>
  {/if}
</svg>

<style>
  .map {
    width: 100%;
    aspect-ratio: 16 / 10;
    border-radius: var(--r-3);
    background: var(--surface);
  }
</style>
