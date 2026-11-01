<!--
@component
VirtualList: renders only the rows near the viewport of a long list (the Activity feed,
the daemon log; spec §6.5, §6.8). Rows keep normal document flow between two spacers, so
the browser's scroll anchoring, find-in-page and focus order work as usual. Row heights
are measured as rows render; unmeasured rows count as `estimate`. It scrolls with its
nearest scrolling ancestor, or the page.
Props:
  - items: readonly T[], in display order.
  - key: (item: T) => string, stable per item.
  - estimate: number, the height in px assumed for a row not yet measured.
  - row: Snippet<[T]>, renders one row.
  - onScroll?: (offset: number) => void, how far the scroller is from its top, in px.
Exports:
  - toTop(): scrolls the scroller back to its top.
-->
<script lang="ts" generics="T">
  import { onMount, type Snippet } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';

  let {
    items,
    key,
    estimate,
    row,
    onScroll,
  }: {
    items: readonly T[];
    key: (item: T) => string;
    estimate: number;
    row: Snippet<[T]>;
    onScroll?: (offset: number) => void;
  } = $props();

  let root: HTMLElement | null = $state(null);
  let scroller: HTMLElement | null = null;
  /** The visible part of the list, in px from its top. */
  let viewTop = $state(0);
  let viewBottom = $state(typeof window === 'undefined' ? 0 : window.innerHeight);
  let viewHeight = $state(typeof window === 'undefined' ? 0 : window.innerHeight);

  const sizes = new SvelteMap<string, number>();

  const offsets = $derived.by(() => {
    const out = new Float64Array(items.length + 1);
    for (let i = 0; i < items.length; i += 1) {
      const item = items[i] as T;
      out[i + 1] = (out[i] ?? 0) + (sizes.get(key(item)) ?? estimate);
    }
    return out;
  });

  /** First index whose bottom is below `y`. */
  function indexAt(y: number): number {
    let lo = 0;
    let hi = items.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if ((offsets[mid + 1] ?? 0) <= y) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  const start = $derived(indexAt(viewTop - viewHeight));
  const end = $derived(Math.min(items.length, indexAt(viewBottom + viewHeight) + 1));
  const before = $derived(offsets[start] ?? 0);
  const after = $derived((offsets[items.length] ?? 0) - (offsets[end] ?? 0));
  const visible = $derived(items.slice(start, end));

  function findScroller(node: HTMLElement): HTMLElement | null {
    for (let el = node.parentElement; el; el = el.parentElement) {
      const overflow = getComputedStyle(el).overflowY;
      if (overflow === 'auto' || overflow === 'scroll') return el;
    }
    return null;
  }

  function measureView(): void {
    if (!root) return;
    const top = root.getBoundingClientRect().top;
    const scrollerTop = scroller ? scroller.getBoundingClientRect().top : 0;
    const height = scroller ? scroller.clientHeight : window.innerHeight;
    viewTop = scrollerTop - top;
    viewBottom = viewTop + height;
    viewHeight = height;
    onScroll?.(scroller ? scroller.scrollTop : window.scrollY);
  }

  const observer =
    typeof ResizeObserver === 'undefined'
      ? null
      : new ResizeObserver((entries) => {
          for (const entry of entries) {
            const id = (entry.target as HTMLElement).dataset['key'];
            const height = entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height;
            if (id !== undefined && sizes.get(id) !== height) sizes.set(id, height);
          }
        });

  function measure(node: HTMLElement) {
    observer?.observe(node);
    return () => observer?.unobserve(node);
  }

  /** Scrolls back to the top of the scroller. */
  export function toTop(): void {
    if (scroller) scroller.scrollTo({ top: 0 });
    else window.scrollTo({ top: 0 });
  }

  onMount(() => {
    if (!root) return;
    scroller = findScroller(root);
    const target: HTMLElement | Window = scroller ?? window;
    let frame = 0;
    const schedule = (): void => {
      if (frame !== 0) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        measureView();
      });
    };
    measureView();
    target.addEventListener('scroll', schedule, { passive: true });
    window.addEventListener('resize', schedule);
    return () => {
      cancelAnimationFrame(frame);
      target.removeEventListener('scroll', schedule);
      window.removeEventListener('resize', schedule);
      observer?.disconnect();
    };
  });
</script>

<div bind:this={root} style:padding-top="{before}px" style:padding-bottom="{after}px">
  {#each visible as item (key(item))}
    <div data-key={key(item)} {@attach measure}>
      {@render row(item)}
    </div>
  {/each}
</div>
