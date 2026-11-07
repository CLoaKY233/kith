<!--
@component
Series (spec §5, §9.1): the detail traffic chart for one device. uPlot, full width, 200 px
tall (160 on mobile). Received above the baseline in the chart ink, sent below in --ink-2,
each the max per slice over an area that fades out toward the baseline. Up to 3 ticks on
the right, hairlines at --line, relative times `−60m … now` below.

Live: the series slides left by one sample over each 2 s tick (it steps under reduced
motion). The y-scale grows at once and shrinks after 10 s of 3× headroom, over 300 ms.
Stale, daemon down or kith-ui gone: it stops sliding and keeps the last data.

Correlation: a 4 px pin on the x-axis for each of the device's journal entries (hover or
focus shows its sentence, click opens Activity); a 4 % band behind every relayed stretch,
labelled `relayed` when wider than 60 px.

Readout pinned on top: `14:32:08 · ↓ 12.4 MB/s ↑ 860 kB/s · direct` under the pointer, the
latest values otherwise. On touch: press and drag; it lingers 2 s after release.

With `at` (from `?at=`, §6.5) the cursor is parked on that moment until the pointer
comes in, so a link from Activity lands on the moment it names.

Screen readers get `role="img"` with a summary, and `Show data` reveals a table of
1-minute buckets.

uPlot loads with the first Series on the page, so it stays out of the first load.
Props:
  - peer: string, the `net/node` key into live.traffic.
  - range: Range ('1m' | '15m' | '1h'), from the URL (see RangeControl).
  - events: readonly Entry[], this device's journal entries, for pins and path history.
  - at?: number | null, server ms to park the cursor on (pick the range with rangeFor).
  - onPin?: (entry: Entry) => void. Defaults to opening Activity for this device.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type uPlot from 'uplot';
  import { go, href } from '../../router';
  import { copy, DOT } from '../copy';
  import { bytes, clock, clockSeconds, rate } from '../format';
  import { live } from '../live/store.svelte';
  import {
    EASING,
    RANGE_MS,
    READOUT_LINGER_MS,
    SPARK_RESCALE_MS,
    TICK_MS,
    TRAFFIC_GAP_MS,
  } from '../live/timing';
  import { readPx, SPACE, watchPalette, type Palette } from '../tokens';
  import type { Range } from '../types';
  import type { Entry } from '../types.gen';
  import Button from '../ui/Button.svelte';
  import Tooltip from '../ui/Tooltip.svelte';
  import { curve, Tween } from './ease';
  import { loadHistory } from './history';
  import { ink, wash } from './ink';
  import { isRelayed, pathAt, pathSpans, pathWord, sentence, type PathSpan } from './events';
  import { bucketCount, latest, minutes, resample } from './resample';
  import { axisRate, GrowShrink, ticks } from './scale';

  let {
    peer,
    range,
    events,
    at = null,
    onPin,
  }: {
    peer: string;
    range: Range;
    events: readonly Entry[];
    at?: number | null;
    onPin?: (entry: Entry) => void;
  } = $props();

  /** §5: line weight of both series. */
  const LINE = 1.5;
  /** The areas under the lines, 16 % of their ink by the plot's edge (hex alpha). */
  const AREA_ALPHA = '29';
  /** §5: the relayed band at 4 % of the ink, labelled when wider than 60 px. */
  const RELAYED_ALPHA = 0.04;
  const RELAYED_LABEL_MIN = 60;
  /** Pins closer than one hit target merge into one. */
  const PIN_HIT = SPACE[5];
  /** The smaller half of the mirror keeps at least a third of the larger, for legibility. */
  const MIRROR_SHARE = 1 / 3;
  /** Below half a pixel of movement a frame is not worth drawing. */
  const MIN_SHIFT_PX = 0.5;
  /** Gridline spacing on the time axis for each range. */
  const X_STEP: Record<Range, number> = {
    '1m': RANGE_MS['1m'],
    '15m': RANGE_MS['15m'] / 3,
    '1h': RANGE_MS['1h'] / 4,
  };
  const ease = curve(EASING.pop);

  type Pin = { key: number; x: number; entries: Entry[]; label: string };
  type Model = {
    xs: number[];
    rx: (number | null | undefined)[];
    tx: (number | null | undefined)[];
  };

  const net = $derived(peer.slice(0, peer.indexOf('/')));
  const node = $derived(peer.slice(peer.indexOf('/') + 1));
  const nameOf = (id: string): string => live.peers[net]?.[id]?.display_name ?? id;
  const name = $derived(nameOf(node));
  const relay = $derived.by(() => {
    const own = live.numbers[peer]?.relay;
    if (own) return nameOf(own);
    const any = live.links[net]?.find((l) => l.relay)?.relay;
    return any ? nameOf(any) : null;
  });
  const currentPath = $derived(live.numbers[peer]?.path ?? live.peers[net]?.[node]?.path ?? null);
  const online = $derived(live.peers[net]?.[node]?.online ?? false);
  const frozen = $derived(live.conn === 'stale' || live.conn === 'daemon_down' || live.conn === 'ui_gone');

  let host: HTMLDivElement | undefined = $state();
  let width = $state(0);
  let mobile = $state(false);
  /** uPlot itself, loaded on mount. */
  let UPlot: typeof uPlot | null = null;
  let plot: uPlot | null = null;
  let palette: Palette | null = null;
  let font = '';

  let model: Model = { xs: [], rx: [], tx: [] };
  let spans: PathSpan[] = [];
  let anchor = 0;
  let arrived = 0;
  let shown = Number.NaN;
  let dirty = true;
  let raf = 0;
  let linger: ReturnType<typeof setTimeout> | null = null;
  const upper = new Tween(0, SPARK_RESCALE_MS, ease);
  const lower = new Tween(0, SPARK_RESCALE_MS, ease);
  const rxScale = new GrowShrink();
  const txScale = new GrowShrink();
  let reduced: MediaQueryList | null = null;
  let scaleKey = '';
  let windowPeak = 0;
  let pointerInside = false;
  /** Width of one slice in ms, for telling a hover over data from one over empty space. */
  let slice: number = TICK_MS;
  /** No sample in the window: no ticks, a dashed baseline (§5 empty state). */
  let empty = $state(true);
  /** The y ticks shown; held while the scale eases so they never sit at stale heights. */
  let heldSplits: number[] = [];
  let history = $state(0);

  /** The row under the cursor, and the cursor's own time for rows that hold a break. */
  let hover = $state<{ i: number | null; t: number } | null>(null);
  let version = $state(0);
  let layer = $state({ left: 0, top: 0, width: 0, height: 0 });
  let pins = $state.raw<Pin[]>([]);
  let labels = $state.raw<{ left: number; width: number }[]>([]);
  let open = $state(false);
  const tableId = $props.id();

  const columns = $derived.by(() => {
    void live.trafficVersion;
    void history;
    return live.traffic.get(peer);
  });
  /** The latest rates; none while stale (§8.5) or once the device stopped reporting. */
  const now = $derived.by(() => {
    void live.trafficVersion;
    if (frozen || !columns) return null;
    const last = latest(columns);
    return last && live.lastTickMs - last.at <= TRAFFIC_GAP_MS ? last : null;
  });

  const windowWord = $derived(copy.chart.window[range]);
  const summary = $derived.by(() => {
    void version;
    if (empty) return copy.chart.emptySummary(name, windowWord);
    const path = online && !frozen && isRelayed(currentPath) ? relay : null;
    return copy.chart.summary(name, windowWord, path, rate(windowPeak), rate(now?.rx ?? null));
  });

  const readout = $derived.by(() => {
    void version;
    if (empty && hover === null) return copy.chart.empty(windowWord);
    const i = hover?.i ?? null;
    if (hover && i === null) {
      const path = pathAt(spans, hover.t);
      return copy.chart.readout(clockSeconds(hover.t), rate(null), rate(null), path ? pathWord(path, relay) : null);
    }
    if (i === null || i >= model.xs.length) {
      const path = online && !frozen ? currentPath : null;
      return copy.chart.readout(
        copy.chart.now,
        rate(now?.rx ?? null),
        rate(now?.tx ?? null),
        path ? pathWord(path, relay) : null,
      );
    }
    const rx = model.rx[i];
    const tx = model.tx[i];
    const t = rx === null ? (hover?.t ?? 0) : (model.xs[i] ?? 0);
    return copy.chart.readout(
      clockSeconds(t),
      rate(rx ?? null),
      rate(tx === null || tx === undefined ? null : -tx),
      ((path) => (path ? pathWord(path, relay) : null))(pathAt(spans, t)),
    );
  });

  const rows = $derived.by(() => {
    if (!open || !columns) return [];
    void live.trafficVersion;
    const to = Math.max(live.lastTickMs, anchor);
    return minutes(columns, to - RANGE_MS[range], to);
  });

  function ratio(): number {
    return UPlot?.pxRatio ?? window.devicePixelRatio ?? 1;
  }

  function readFont(): string {
    const micro = getComputedStyle(document.documentElement).getPropertyValue('--t-micro').trim();
    return micro.replace(/\s*\/\s*[\d.]+px/, '');
  }

  function measure(texts: readonly (string | null)[] | null): number {
    const ctx = document.createElement('canvas').getContext('2d');
    if (!ctx || !texts) return SPACE[4];
    ctx.font = font;
    let w = 0;
    for (const t of texts) if (t) w = Math.max(w, ctx.measureText(t).width);
    return Math.ceil(w) + SPACE[2] + SPACE[1];
  }

  /** Recomputes slices, scales and path history from the store. */
  function rebuild(): void {
    const cols = live.traffic.get(peer);
    const rangeMs = RANGE_MS[range];
    const last = cols && cols.length > 0 ? (cols.at[cols.length - 1] ?? 0) : 0;
    const next = Math.max(live.lastTickMs, last);
    if (next !== anchor) {
      anchor = next;
      arrived = performance.now();
    }
    const plotWidth = plot ? plot.bbox.width / ratio() : width;
    const n = bucketCount(plotWidth, rangeMs);
    slice = Math.max(TICK_MS, Math.ceil(rangeMs / n / TICK_MS) * TICK_MS);
    const from = Math.floor((anchor - rangeMs - TICK_MS) / slice) * slice;
    const to = Math.ceil(anchor / slice) * slice;
    const b = cols ? resample(cols, from, to, Math.max(1, Math.round((to - from) / slice))) : null;

    const m: Model = { xs: [], rx: [], tx: [] };
    let rxPeak = 0;
    let txPeak = 0;
    if (b) {
      for (let i = 0; i < b.count; i += 1) {
        const rx = b.rxMax[i] ?? Number.NaN;
        const tx = b.txMax[i] ?? Number.NaN;
        if (Number.isNaN(rx) && Number.isNaN(tx)) continue;
        const t = b.at[i] ?? 0;
        if (b.breakBefore[i] && m.xs.length > 0) {
          m.xs.push(((m.xs[m.xs.length - 1] ?? t) + t) / 2);
          m.rx.push(null);
          m.tx.push(null);
        }
        m.xs.push(t);
        m.rx.push(Number.isNaN(rx) ? undefined : rx);
        m.tx.push(Number.isNaN(tx) ? undefined : -tx);
        if (t >= anchor - rangeMs) {
          if (rx > rxPeak) rxPeak = rx;
          if (tx > txPeak) txPeak = tx;
        }
      }
    }
    model = m;
    windowPeak = rxPeak;
    empty = !m.xs.some((t, i) => t >= anchor - rangeMs && (m.rx[i] != null || m.tx[i] != null));
    // Path history reaches back only as far as data or the journal can back it.
    const firstSample = m.xs.find((_, i) => m.rx[i] != null || m.tx[i] != null) ?? anchor;
    const firstChange = events.reduce(
      (min, e) => ('node' in e && e.node === node ? Math.min(min, e.at_ms) : min),
      anchor,
    );
    spans = pathSpans(events, node, currentPath, Math.max(from, Math.min(firstSample, firstChange)), anchor);

    const t = performance.now();
    const key = `${peer} ${range}`;
    if (key !== scaleKey) {
      scaleKey = key;
      rxScale.reset();
      txScale.reset();
      upper.set(0, t, true);
      lower.set(0, t, true);
    }
    const up = rxScale.update(rxPeak, t);
    const down = txScale.update(txPeak, t);
    const instant = !(up.animate || down.animate) || (reduced?.matches ?? false) || upper.target === 0;
    upper.set(Math.max(up.top, down.top * MIRROR_SHARE), t, instant);
    lower.set(Math.max(down.top, up.top * MIRROR_SHARE), t, instant);

    plot?.setData([m.xs, m.rx, m.tx], false);
    dirty = true;
    version += 1;
    request();
  }

  function request(): void {
    if (raf === 0) raf = requestAnimationFrame(frame);
  }

  function frame(): void {
    raf = 0;
    if (!plot) return;
    const t = performance.now();
    const rangeMs = RANGE_MS[range];
    const still = (reduced?.matches ?? false) || frozen;
    const lag = still ? 0 : Math.max(0, TICK_MS - (t - arrived));
    const right = anchor - lag;
    const moving = upper.moving(t) || lower.moving(t);
    const pxPerMs = plot.bbox.width / ratio() / rangeMs;
    if (dirty || moving || Number.isNaN(shown) || Math.abs(right - shown) * pxPerMs >= MIN_SHIFT_PX) {
      dirty = false;
      shown = right;
      const p = plot;
      p.batch(() => {
        p.setScale('x', { min: right - rangeMs, max: right });
        p.setScale('y', { min: -lower.value(t), max: upper.value(t) });
      });
    }
    park();
    if (lag > 0 || moving) request();
  }

  /** Puts the cursor on `at` while the pointer is elsewhere; hides it once `at` scrolls out. */
  function park(): void {
    if (!plot || pointerInside || linger !== null || at === null) return;
    const px = plot.valToPos(at, 'x');
    const inside = px >= 0 && px <= plot.bbox.width / ratio();
    plot.setCursor({ left: inside ? px : -1, top: inside ? plot.bbox.height / ratio() / 2 : -1 });
  }

  function x(u: uPlot, v: number): number {
    return u.valToPos(v, 'x', true);
  }

  function drawRelayed(u: uPlot): void {
    if (!palette) return;
    const { left, top, width: w, height: h } = u.bbox;
    const ctx = u.ctx;
    ctx.save();
    ctx.globalAlpha = RELAYED_ALPHA;
    ctx.fillStyle = palette['chart-ink'];
    for (const span of spans) {
      if (!isRelayed(span.path)) continue;
      const x0 = Math.max(left, x(u, span.from));
      const x1 = Math.min(left + w, x(u, span.to));
      if (x1 > x0) ctx.fillRect(x0, top, x1 - x0, h);
    }
    ctx.restore();
  }

  /** The baseline: a hairline, dashed while the window has no samples (§5 empty state). */
  function drawBaseline(u: uPlot): void {
    if (!palette) return;
    const { left, width: w } = u.bbox;
    const ctx = u.ctx;
    const r = ratio();
    const y0 = Math.round(u.valToPos(0, 'y', true)) + 0.5;
    ctx.save();
    ctx.strokeStyle = empty ? ink(palette, 'ink-4') : palette.line;
    ctx.lineWidth = Math.max(1, readPx('hairline') * r);
    if (empty) ctx.setLineDash([SPACE[1] * r, SPACE[1] * r]);
    ctx.beginPath();
    ctx.moveTo(left, y0);
    ctx.lineTo(left + w, y0);
    ctx.stroke();
    ctx.restore();
  }

  /** Received fades down from the plot's top, sent up from its bottom, both to the baseline. */
  function area(edge: 'top' | 'bottom', token: 'chart-ink' | 'ink-2') {
    return (u: uPlot): CanvasGradient | string => {
      const { top, height } = u.bbox;
      const y0 = u.valToPos(0, 'y', true);
      // Before the first data the scale is empty and y0 is not a number.
      if (!palette || !Number.isFinite(y0)) return 'transparent';
      return wash(u.ctx, ink(palette, token), AREA_ALPHA, edge === 'top' ? top : top + height, y0);
    };
  }

  function syncOverlay(u: uPlot): void {
    const r = ratio();
    const { left, top, width: w, height: h } = u.bbox;
    layer = { left: left / r, top: top / r, width: w / r, height: h / r };
    const min = u.scales['x']?.min ?? 0;
    const max = u.scales['x']?.max ?? 0;
    const relayName = relay;

    const next: Pin[] = [];
    const visible = events
      .filter((e) => e.at_ms >= min && e.at_ms <= max)
      .sort((a, b) => a.at_ms - b.at_ms || a.seq - b.seq);
    for (const e of visible) {
      const px = u.valToPos(e.at_ms, 'x');
      const last = next[next.length - 1];
      if (last && px - last.x < PIN_HIT) last.entries.push(e);
      else next.push({ key: e.seq, x: px, entries: [e], label: '' });
    }
    for (const pin of next) pin.label = pin.entries.map((e) => sentence(e, nameOf, relayName)).join(DOT);
    pins = next;

    const nextLabels: { left: number; width: number }[] = [];
    for (const span of spans) {
      if (!isRelayed(span.path)) continue;
      const x0 = Math.max(0, u.valToPos(span.from, 'x'));
      const x1 = Math.min(w / r, u.valToPos(span.to, 'x'));
      if (x1 - x0 > RELAYED_LABEL_MIN) nextLabels.push({ left: x0, width: x1 - x0 });
    }
    labels = nextLabels;
  }

  function xSplits(u: uPlot): number[] {
    const max = u.scales['x']?.max ?? 0;
    const min = u.scales['x']?.min ?? 0;
    const step = X_STEP[range];
    const out: number[] = [];
    for (let v = max; v >= min - 1; v -= step) out.unshift(v);
    return out;
  }

  function xValues(u: uPlot, splits: number[]): string[] {
    const max = u.scales['x']?.max ?? 0;
    return splits.map((v) => {
      const m = Math.round((max - v) / RANGE_MS['1m']);
      return m === 0 ? copy.chart.now : copy.chart.minutesBack(m);
    });
  }

  function ySplits(): number[] {
    if (empty) return [];
    const t = performance.now();
    if (heldSplits.length === 0 || !(upper.moving(t) || lower.moving(t))) {
      heldSplits = ticks(upper.target, lower.target).sort((a, b) => a - b);
    }
    return heldSplits;
  }

  function create(p: Palette): void {
    if (!host || !UPlot) return;
    plot?.destroy();
    font = readFont();
    const height = readPx(mobile ? 'series-h-mobile' : 'series-h');
    const hairline = readPx('hairline');
    const opts: uPlot.Options = {
      width: Math.max(1, width),
      height,
      padding: [SPACE[2], 0, 0, SPACE[4]],
      legend: { show: false },
      cursor: {
        y: false,
        drag: { x: false, y: false, setScale: false },
        points: { size: SPACE[2], width: LINE, fill: p.surface },
      },
      select: { show: false, left: 0, top: 0, width: 0, height: 0 },
      scales: { x: { time: false, auto: false }, y: { auto: false } },
      series: [
        {},
        { stroke: p['chart-ink'], fill: area('top', 'chart-ink'), width: LINE, points: { show: false } },
        { stroke: ink(p, 'ink-2'), fill: area('bottom', 'ink-2'), width: LINE, points: { show: false } },
      ],
      axes: [
        {
          side: 2,
          stroke: ink(p, 'ink-3'),
          font,
          size: SPACE[5],
          gap: SPACE[1] + SPACE[1],
          grid: { show: false },
          ticks: { show: false },
          border: { show: false },
          splits: xSplits,
          values: xValues,
        },
        {
          side: 1,
          stroke: ink(p, 'ink-3'),
          font,
          gap: SPACE[2],
          size: (_u, values) => measure(values),
          grid: { stroke: p.line, width: hairline },
          ticks: { show: false },
          border: { show: false },
          splits: ySplits,
          values: (_u, splits) => splits.map(axisRate),
        },
      ],
      hooks: {
        drawClear: [drawRelayed],
        drawAxes: [drawBaseline],
        draw: [syncOverlay],
        setCursor: [
          (u) => {
            const i = u.cursor.idx;
            const left = u.cursor.left ?? -1;
            if (left < 0) {
              hover = null;
              return;
            }
            const t = u.posToVal(left, 'x');
            const near = i !== null && i !== undefined && Math.abs((model.xs[i] ?? Infinity) - t) <= slice;
            hover = { i: near ? i : null, t };
            u.root.classList.toggle('far', !near);
          },
        ],
      },
    };
    plot = new UPlot(opts, [model.xs, model.rx, model.tx], host);
    const over = plot.over;
    over.addEventListener('pointerdown', touchStart);
    over.addEventListener('pointermove', touchMove);
    over.addEventListener('pointerup', touchEnd);
    over.addEventListener('pointercancel', touchEnd);
    over.addEventListener('pointerenter', enter);
    over.addEventListener('pointerleave', leave);
    shown = Number.NaN;
    rebuild();
  }

  function enter(event: PointerEvent): void {
    if (event.pointerType !== 'touch') pointerInside = true;
  }

  function leave(event: PointerEvent): void {
    if (event.pointerType === 'touch') return;
    pointerInside = false;
    dirty = true;
    request();
  }

  function touchAt(event: PointerEvent): void {
    if (!plot) return;
    const rect = plot.over.getBoundingClientRect();
    plot.setCursor({ left: event.clientX - rect.left, top: event.clientY - rect.top });
  }

  function touchStart(event: PointerEvent): void {
    if (event.pointerType !== 'touch' || !plot) return;
    if (linger !== null) clearTimeout(linger);
    linger = null;
    plot.over.setPointerCapture(event.pointerId);
    touchAt(event);
  }

  function touchMove(event: PointerEvent): void {
    if (event.pointerType !== 'touch' || !plot?.over.hasPointerCapture(event.pointerId)) return;
    touchAt(event);
  }

  function touchEnd(event: PointerEvent): void {
    if (event.pointerType !== 'touch') return;
    linger = setTimeout(() => {
      linger = null;
      plot?.setCursor({ left: -1, top: -1 });
      park();
    }, READOUT_LINGER_MS);
  }

  function openPin(pin: Pin): void {
    const first = pin.entries[0];
    if (!first) return;
    if (onPin) onPin(first);
    else go(`${href({ name: 'activity', net })}?device=${encodeURIComponent(node)}`);
  }

  onMount(() => {
    reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
    const narrow = window.matchMedia('(width < 40rem)');
    mobile = narrow.matches;
    const onNarrow = (): void => {
      mobile = narrow.matches;
      if (palette) create(palette);
    };
    narrow.addEventListener('change', onNarrow);
    const unwatch = watchPalette((p) => {
      palette = p;
      create(p);
    });
    const fonts = document.fonts;
    const redraw = (): void => {
      if (palette) create(palette);
    };
    fonts.addEventListener('loadingdone', redraw);
    let alive = true;
    void loadHistory(live.traffic).then(() => {
      if (alive) history += 1;
    });
    void import('uplot').then(
      (module) => {
        if (!alive) return;
        UPlot = module.default;
        redraw();
      },
      () => undefined,
    );
    return () => {
      alive = false;
      narrow.removeEventListener('change', onNarrow);
      fonts.removeEventListener('loadingdone', redraw);
      unwatch();
      if (raf !== 0) cancelAnimationFrame(raf);
      if (linger !== null) clearTimeout(linger);
      plot?.destroy();
      plot = null;
    };
  });

  $effect(() => {
    if (!plot || width <= 0) return;
    plot.setSize({ width, height: readPx(mobile ? 'series-h-mobile' : 'series-h') });
    untrack(rebuild);
  });

  $effect(() => {
    void live.trafficVersion;
    void history;
    void events;
    void range;
    void peer;
    void currentPath;
    if (plot) untrack(rebuild);
  });

  $effect(() => {
    void frozen;
    void at;
    dirty = true;
    if (plot) request();
  });
</script>

<figure class="series">
  <p class="readout">{readout}</p>
  <div class="frame" bind:clientWidth={width}>
    <div class="plot" role="img" aria-label={summary} bind:this={host}></div>
    <div
      class="layer"
      style:left="{layer.left}px"
      style:top="{layer.top}px"
      style:width="{layer.width}px"
      style:height="{layer.height}px"
    >
      {#each labels as label (label.left)}
        <span class="relayed" aria-hidden="true" style:left="{label.left}px" style:max-width="{label.width}px">
          {copy.chart.relayed}
        </span>
      {/each}
      {#each pins as pin (pin.key)}
        <Tooltip text={pin.label}>
          {#snippet trigger(props)}
            <button
              {...props}
              type="button"
              class="pin"
              style:left="{pin.x}px"
              aria-label={pin.label}
              onclick={(event) => {
                const own = props['onclick'];
                if (typeof own === 'function') own(event);
                openPin(pin);
              }}
            ></button>
          {/snippet}
        </Tooltip>
      {/each}
    </div>
  </div>
  <div class="data">
    <Button variant="ghost" aria-expanded={open} aria-controls={tableId} onclick={() => (open = !open)}>
      {open ? copy.chart.hideData : copy.chart.showData}
    </Button>
    {#if open}
      <table id={tableId}>
        <caption>{summary}</caption>
        <thead>
          <tr>
            <th scope="col">{copy.chart.tableTime}</th>
            <th scope="col">{copy.chart.tableReceived}</th>
            <th scope="col">{copy.chart.tableSent}</th>
          </tr>
        </thead>
        <tbody>
          {#each rows as row (row.start)}
            <tr>
              <th scope="row"><time datetime={new Date(row.start).toISOString()}>{clock(row.start)}</time></th>
              <td>{row.rx === null ? copy.units.none : bytes(row.rx)}</td>
              <td>{row.tx === null ? copy.units.none : bytes(row.tx)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</figure>

<style>
  .series {
    display: grid;
    gap: var(--s-2);
    margin: 0;
    min-width: 0;
  }
  .readout {
    margin: 0;
    font: var(--t-small);
    font-variant-numeric: tabular-nums slashed-zero;
    color: var(--ink-1);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .frame {
    position: relative;
    height: var(--series-h);
    min-width: 0;
  }
  .plot {
    position: absolute;
    inset: 0;
  }
  .layer {
    position: absolute;
    pointer-events: none;
  }
  .relayed {
    position: absolute;
    top: var(--s-1);
    padding-left: var(--s-1);
    overflow: hidden;
    white-space: nowrap;
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    color: var(--ink-3);
  }
  .pin {
    --pin-hit: var(--s-5);
    position: absolute;
    top: calc(100% + var(--s-1) / 2 - var(--pin-hit) / 2);
    width: var(--pin-hit);
    height: var(--pin-hit);
    margin-left: calc(var(--pin-hit) / -2);
    padding: 0;
    border: 0;
    border-radius: var(--r-1);
    background: none;
    cursor: pointer;
    pointer-events: auto;
  }
  .pin::after {
    content: '';
    position: absolute;
    left: 50%;
    top: 50%;
    width: calc(2 * var(--hairline));
    height: var(--s-1);
    transform: translate(-50%, -50%);
    background: var(--ink-2);
    transition: background-color var(--d-hover) var(--e-linear);
  }
  .pin:hover::after,
  .pin:focus-visible::after {
    background: var(--ink-1);
  }
  .pin:focus-visible {
    outline: var(--focus-ring) solid var(--accent);
    outline-offset: calc(var(--focus-offset) * -1);
  }
  @media (forced-colors: active) {
    .pin::after {
      forced-color-adjust: none;
      background: var(--chart-ink);
    }
  }
  @media (pointer: coarse) {
    .pin {
      --pin-hit: var(--target-touch);
    }
  }
  .data {
    display: grid;
    justify-items: start;
    gap: var(--s-2);
  }
  .data > :global(button) {
    margin-inline-start: calc(-1 * var(--s-3));
  }
  .frame :global(.far .u-cursor-pt) {
    display: none;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font: var(--t-small);
  }
  caption {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  thead th {
    font: var(--t-micro);
    letter-spacing: var(--t-micro-tracking);
    font-variant-caps: all-small-caps;
    color: var(--ink-3);
    text-align: right;
  }
  th,
  td {
    padding: var(--s-1) var(--s-2);
    border-bottom: var(--hairline) solid var(--line);
    text-align: right;
  }
  th:first-child {
    text-align: left;
    font-weight: inherit;
  }
  td {
    color: var(--ink-1);
  }

  .frame :global(.u-wrap) {
    position: relative;
    user-select: none;
  }
  .frame :global(.u-over),
  .frame :global(.u-under) {
    position: absolute;
  }
  .frame :global(.u-over) {
    touch-action: pan-y;
  }
  .frame :global(.u-under) {
    overflow: hidden;
  }
  .frame :global(canvas) {
    display: block;
    position: relative;
    width: 100%;
    height: 100%;
  }
  .frame :global(.u-axis) {
    position: absolute;
  }
  .frame :global(.u-cursor-x) {
    position: absolute;
    left: 0;
    top: 0;
    height: 100%;
    border-right: var(--hairline) solid var(--line-strong);
    pointer-events: none;
    will-change: transform;
  }
  .frame :global(.u-cursor-pt) {
    position: absolute;
    top: 0;
    left: 0;
    border-radius: var(--r-full);
    border: 0 solid;
    pointer-events: none;
    will-change: transform;
    background-clip: padding-box !important;
  }
  .frame :global(.u-off) {
    display: none;
  }
  @media (width < 40rem) {
    .frame {
      height: var(--series-h-mobile);
    }
  }
</style>
