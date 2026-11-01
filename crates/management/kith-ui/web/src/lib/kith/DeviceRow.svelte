<!--
@component
DeviceRow (spec §5): the core 44 px row. Placeholder; workstream D owns it.
Props:
  - net: string, network id.
  - peer: PeerInfo.
  - self?: boolean, this device (pinned first, tagged `you`).
  - selected?: boolean.
  - onOpen?: () => void.
-->
<script lang="ts">
  import { copy } from '../copy';
  import type { PeerInfo } from '../types.gen';
  import StatusDot from '../ui/StatusDot.svelte';

  let {
    net,
    peer,
    self = false,
    selected = false,
    onOpen,
  }: { net: string; peer: PeerInfo; self?: boolean; selected?: boolean; onOpen?: () => void } =
    $props();
</script>

<div class="row" role="row" aria-selected={selected} data-net={net} tabindex="-1" onclick={onOpen} onkeydown={(e) => e.key === 'Enter' && onOpen?.()}>
  <StatusDot state={peer.online ? 'online' : 'offline'} />
  <span>{peer.display_name}</span>
  {#if self}<span class="you">{copy.device.you}</span>{/if}
  <span class="addr">{peer.address}</span>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: var(--row);
  }
  .addr {
    font: var(--t-small);
    font-family: var(--ff-mono);
    color: var(--ink-3);
  }
  .you {
    color: var(--ink-3);
  }
</style>
