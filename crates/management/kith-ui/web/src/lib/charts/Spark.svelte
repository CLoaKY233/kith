<!--
@component
Spark (spec §5): received traffic for one device over the last 5 minutes, 120×24, the
same on every row; phone rows leave it out so their numbers fit. Painted by the list's
shared painter in the tick's frame. Hover (mouse and pen only) shows a crosshair and
`14:02:16 · ↓ 2.1 MB/s · peak 4.0 MB/s`. Empty draws a dashed baseline; offline, or a
stale page, freezes it in --ink-4. Hidden from screen readers: Rate and the row's
accessible name carry the numbers.
Props:
  - peer: string, the `net/node` key into live.traffic.
  - frozen?: boolean. Defaults to "offline, or the page is stale or the daemon down".
  - painter?: SparkPainter. Defaults to the page's shared painter.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { copy } from '../copy';
  import { clockSeconds, rate } from '../format';
  import { live } from '../live/store.svelte';
  import { SPACE } from '../tokens';
  import { sparkPainter, type SparkHandle, type SparkPainter, type SparkPoint } from './sparkPainter';

  let {
    peer,
    frozen,
    painter = sparkPainter(),
  }: { peer: string; frozen?: boolean; painter?: SparkPainter } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let handle = $state.raw<SparkHandle | null>(null);
  let tip = $state<SparkPoint | null>(null);

  const offline = $derived.by(() => {
    const cut = peer.indexOf('/');
    const info = live.peers[peer.slice(0, cut)]?.[peer.slice(cut + 1)];
    return info ? !info.online : false;
  });
  const isFrozen = $derived(
    frozen ?? (offline || live.conn === 'stale' || live.conn === 'daemon_down' || live.conn === 'ui_gone'),
  );
  const tipText = $derived(
    tip ? copy.chart.sparkTip(clockSeconds(tip.at), rate(tip.rate), rate(tip.peak)) : '',
  );

  $effect(() => {
    if (!canvas) return;
    const h = painter.register(canvas, untrack(() => peer), (next) => (tip = next));
    handle = h;
    const seen = new IntersectionObserver(([entry]) => h.visible(entry?.isIntersecting ?? true));
    seen.observe(canvas);
    return () => {
      seen.disconnect();
      h.dispose();
      handle = null;
    };
  });

  $effect(() => {
    handle?.update(peer, isFrozen);
  });

  $effect(() => {
    void live.trafficVersion;
    painter.request();
  });

  function move(event: PointerEvent): void {
    if (event.pointerType === 'touch') return;
    handle?.hover(event.offsetX);
  }

  let box = $state<HTMLSpanElement | null>(null);
  let tipW = $state(0);
  /** The tip's centre, kept inside the viewport with a gutter. */
  const tipLeft = $derived.by(() => {
    if (!tip) return 0;
    const origin = box?.getBoundingClientRect().left ?? 0;
    const gutter = SPACE[3];
    const view = typeof window === 'undefined' ? Infinity : window.innerWidth;
    const centre = Math.max(origin + tip.x, gutter + tipW / 2);
    return Math.min(centre, view - gutter - tipW / 2) - origin;
  });

  function leave(): void {
    handle?.hover(null);
  }
</script>

<span class="spark" bind:this={box}>
  <canvas bind:this={canvas} aria-hidden="true" onpointermove={move} onpointerleave={leave}></canvas>
  {#if tip}
    <span class="tip" bind:offsetWidth={tipW} style:left="{tipLeft}px">{tipText}</span>
  {/if}
</span>

<style>
  .spark {
    position: relative;
    display: block;
    width: var(--spark-w);
    height: var(--spark-h);
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .tip {
    position: absolute;
    bottom: calc(100% + var(--s-1));
    z-index: var(--z-popover);
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-1);
    background: var(--ink-1);
    color: var(--surface);
    font: var(--t-small);
    white-space: nowrap;
    pointer-events: none;
    transform: translateX(-50%);
  }
</style>
