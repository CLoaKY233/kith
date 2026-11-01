// One shared Canvas 2D painter for a list of sparks (spec §5 Spark). Every Spark registers
// its canvas here; one requestAnimationFrame repaints them all after a tick, so a list of
// 60 rows costs one frame, not 60.
//
// Each row: received traffic only, as an area at SPARK_AREA_ALPHA of the chart ink under a
// SPARK_LINE px line, on its own y-scale (scale.ts). Frozen rows (offline, or the page is
// stale) draw in --ink-4 and stop advancing. A row with no rates draws a dashed baseline.

import { live } from '../live/store.svelte';
import { EASING, SPARK_RESCALE_MS, TICK_MS } from '../live/timing';
import { watchPalette, type Palette } from '../tokens';
import type { TrafficColumns } from '../types';
import { curve, Tween } from './ease';
import { ink } from './ink';
import { bucketCount, peak, resample, type Buckets } from './resample';
import { sparkFit, sparkTop } from './scale';

/** §5: rx area at 12 % of the ink under a 1.25 px line. */
const SPARK_AREA_ALPHA = 0.12;
const SPARK_LINE = 1.25;
const CROSSHAIR = 1;
const EMPTY_DASH = [2, 2];
/** The window a spark shows: the 150 samples of the boot snapshot's traffic tail. */
const SPARK_SAMPLES = 150;
const SPARK_WINDOW_MS = SPARK_SAMPLES * TICK_MS;

/** What the hover tooltip shows (`14:02:16 · ↓ 2.1 MB/s · peak 4.0 MB/s`). */
export type SparkPoint = {
  /** Crosshair position from the canvas's left edge, CSS px. */
  readonly x: number;
  readonly at: number;
  /** Bytes per second in the hovered slice, null when it has none. */
  readonly rate: number | null;
  readonly peak: number;
};

/** Where the painter reads traffic, and the right edge of a live row. */
export interface SparkSource {
  columns(peer: string): TrafficColumns | undefined;
  /** Server time of the latest tick. */
  anchor(): number;
}

export interface SparkHandle {
  /** Changes the row's device or its frozen state. */
  update(peer: string, frozen: boolean): void;
  /** Sets the pointer's x in CSS px, or null when it leaves. */
  hover(x: number | null): void;
  /** Skips painting while the canvas is scrolled out of view. */
  visible(on: boolean): void;
  dispose(): void;
}

export interface SparkPainter {
  /** Adds a canvas; `onTip` gets the hovered point after every paint while hovering. */
  register(canvas: HTMLCanvasElement, peer: string, onTip: (tip: SparkPoint | null) => void): SparkHandle;
  /** Repaints every visible spark in the next animation frame. */
  request(): void;
}

type Slot = {
  canvas: HTMLCanvasElement;
  peer: string;
  frozen: boolean;
  visible: boolean;
  width: number;
  height: number;
  fit: number;
  top: Tween;
  hoverX: number | null;
  onTip: (tip: SparkPoint | null) => void;
};

const ease = curve(EASING.pop);

function reducedMotion(): boolean {
  return typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

function lastAt(columns: TrafficColumns | undefined): number {
  return columns && columns.length > 0 ? (columns.at[columns.length - 1] ?? 0) : 0;
}

function drawEmpty(ctx: CanvasRenderingContext2D, slot: Slot, palette: Palette): void {
  const y = slot.height - SPARK_LINE / 2;
  ctx.strokeStyle = ink(palette, 'ink-4');
  ctx.lineWidth = 1;
  ctx.setLineDash(EMPTY_DASH);
  ctx.beginPath();
  ctx.moveTo(0, y);
  ctx.lineTo(slot.width, y);
  ctx.stroke();
  ctx.setLineDash([]);
}

/** Runs of slices joined by a line: NaN slices are skipped, breaks start a new run. */
function runs(b: Buckets): number[][] {
  const out: number[][] = [];
  let run: number[] = [];
  for (let i = 0; i < b.count; i += 1) {
    if (Number.isNaN(b.rxMax[i] ?? Number.NaN)) continue;
    if (b.breakBefore[i] && run.length > 0) {
      out.push(run);
      run = [];
    }
    run.push(i);
  }
  if (run.length > 0) out.push(run);
  return out;
}

function drawLine(
  ctx: CanvasRenderingContext2D,
  slot: Slot,
  b: Buckets,
  top: number,
  color: string,
): void {
  const base = slot.height - SPARK_LINE / 2;
  const usable = base - SPARK_LINE / 2;
  const step = slot.width / b.count;
  const x = (i: number): number => (i + 0.5) * step;
  const y = (i: number): number =>
    base - Math.min(1, Math.max(0, (b.rxMax[i] ?? 0) / top)) * usable;
  ctx.fillStyle = color;
  ctx.strokeStyle = color;
  ctx.lineWidth = SPARK_LINE;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  for (const run of runs(b)) {
    const first = run[0] ?? 0;
    const last = run[run.length - 1] ?? 0;
    // A lone slice still shows as a short flat mark one slice wide.
    const x0 = run.length === 1 ? x(first) - step / 2 : x(first);
    const x1 = run.length === 1 ? x(first) + step / 2 : x(last);
    const line = new Path2D();
    line.moveTo(x0, y(first));
    for (const i of run) line.lineTo(x(i), y(i));
    line.lineTo(x1, y(last));
    const area = new Path2D(line);
    area.lineTo(x1, base);
    area.lineTo(x0, base);
    area.closePath();
    ctx.globalAlpha = SPARK_AREA_ALPHA;
    ctx.fill(area);
    ctx.globalAlpha = 1;
    ctx.stroke(line);
  }
}

function nearest(b: Buckets, x: number, width: number): number {
  return Math.min(b.count - 1, Math.max(0, Math.floor((x / width) * b.count)));
}

export function createSparkPainter(source: SparkSource): SparkPainter {
  const slots = new Set<Slot>();
  let palette: Palette | null = null;
  let unwatch: (() => void) | null = null;
  let frame = 0;

  function paint(slot: Slot, now: number): boolean {
    const ctx = slot.canvas.getContext('2d');
    if (!ctx || !palette || slot.width <= 0 || slot.height <= 0) return false;
    const dpr = window.devicePixelRatio || 1;
    const pw = Math.round(slot.width * dpr);
    const ph = Math.round(slot.height * dpr);
    if (slot.canvas.width !== pw) slot.canvas.width = pw;
    if (slot.canvas.height !== ph) slot.canvas.height = ph;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, slot.width, slot.height);

    const columns = source.columns(slot.peer);
    const right = slot.frozen ? lastAt(columns) : Math.max(source.anchor(), lastAt(columns));
    const b = columns
      ? resample(columns, right - SPARK_WINDOW_MS, right, bucketCount(slot.width, SPARK_WINDOW_MS))
      : null;
    const high = b ? peak(b.rxMax) : 0;
    if (!b || runs(b).length === 0) {
      drawEmpty(ctx, slot, palette);
      slot.onTip(null);
      return false;
    }

    const fit = sparkFit(slot.fit, high);
    if (fit !== slot.fit) {
      slot.top.set(sparkTop(fit), now, slot.fit === 0 || reducedMotion());
      slot.fit = fit;
    }
    const color = slot.frozen ? ink(palette, 'ink-4') : palette['chart-ink'];
    drawLine(ctx, slot, b, slot.top.value(now), color);

    if (slot.hoverX === null) {
      slot.onTip(null);
    } else {
      const i = nearest(b, slot.hoverX, slot.width);
      const cx = Math.round(((i + 0.5) * slot.width) / b.count) + CROSSHAIR / 2;
      ctx.strokeStyle = ink(palette, 'line-strong');
      ctx.lineWidth = CROSSHAIR;
      ctx.beginPath();
      ctx.moveTo(cx, 0);
      ctx.lineTo(cx, slot.height);
      ctx.stroke();
      const v = b.rxMax[i] ?? Number.NaN;
      slot.onTip({ x: cx, at: b.at[i] ?? right, rate: Number.isNaN(v) ? null : v, peak: high });
    }
    return slot.top.moving(now);
  }

  function run(): void {
    frame = 0;
    const now = performance.now();
    let again = false;
    for (const slot of slots) if (slot.visible && paint(slot, now)) again = true;
    if (again) request();
  }

  function request(): void {
    if (frame !== 0 || typeof requestAnimationFrame === 'undefined') return;
    frame = requestAnimationFrame(run);
  }

  return {
    request,
    register(canvas, peer, onTip) {
      const slot: Slot = {
        canvas,
        peer,
        frozen: false,
        visible: true,
        width: canvas.clientWidth,
        height: canvas.clientHeight,
        fit: 0,
        top: new Tween(0, SPARK_RESCALE_MS, ease),
        hoverX: null,
        onTip,
      };
      slots.add(slot);
      unwatch ??= watchPalette((next) => {
        palette = next;
        request();
      });
      const resize = new ResizeObserver(([entry]) => {
        if (!entry) return;
        slot.width = entry.contentRect.width;
        slot.height = entry.contentRect.height;
        request();
      });
      resize.observe(canvas);
      request();
      return {
        update(nextPeer, frozen) {
          if (nextPeer !== slot.peer) slot.fit = 0;
          slot.peer = nextPeer;
          slot.frozen = frozen;
          request();
        },
        hover(x) {
          slot.hoverX = x;
          request();
        },
        visible(on) {
          slot.visible = on;
          if (on) request();
        },
        dispose() {
          resize.disconnect();
          slots.delete(slot);
          if (slots.size === 0 && unwatch) {
            unwatch();
            unwatch = null;
            palette = null;
          }
        },
      };
    },
  };
}

let shared: SparkPainter | null = null;

/** The page's painter, reading the live store. Lists that want their own make one. */
export function sparkPainter(): SparkPainter {
  shared ??= createSparkPainter({
    columns: (peer) => live.traffic.get(peer),
    anchor: () => live.lastTickMs,
  });
  return shared;
}
