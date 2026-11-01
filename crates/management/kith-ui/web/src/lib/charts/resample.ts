// Traffic counters to rates, resampled for drawing (spec §9.1).
//
// Rate is Δbytes/Δt between consecutive samples, placed at the later sample. A negative
// delta is a counter reset: that rate is dropped, and the line carries on over it. An
// interval longer than TRAFFIC_GAP_MS is a gap: the line breaks there and never joins the
// two sides. A sample closer than half a tick to the last one folds into the next interval,
// so two samples taken moments apart never read as a spike. Buckets are slices of time, so
// missing samples never shift the x-axis.

import { RANGE_MS, TICK_MS, TRAFFIC_GAP_MS } from '../live/timing';
import type { TrafficColumns } from '../types';

/** Rates resampled into equal slices of time. NaN marks a slice with no rate. */
export interface Buckets {
  readonly from: number;
  readonly to: number;
  readonly count: number;
  /** Time of the slice's latest rate, ms; its centre while it has none. */
  readonly at: Float64Array;
  readonly rxMax: Float64Array;
  readonly rxMin: Float64Array;
  readonly txMax: Float64Array;
  readonly txMin: Float64Array;
  /** 1 when a gap lies between this slice and the previous slice with a rate. */
  readonly breakBefore: Uint8Array;
}

/** Bytes per second at one moment. */
export type Rates = { readonly at: number; readonly rx: number; readonly tx: number };

/**
 * W/2 slices for a W px wide chart, but never thinner than one sample, so a steady stream
 * of samples fills every slice.
 */
export function bucketCount(widthPx: number, rangeMs: number): number {
  const byWidth = Math.floor(widthPx / 2);
  const bySamples = Math.floor(rangeMs / TICK_MS);
  return Math.max(1, Math.min(byWidth, bySamples));
}

/** Index of the first sample later than `t`. */
function firstAfter(at: Float64Array, length: number, t: number): number {
  let lo = 0;
  let hi = length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if ((at[mid] ?? 0) > t) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

const MIN_INTERVAL_MS = TICK_MS / 2;

function rate(values: Float64Array, from: number, to: number, dt: number): number {
  const dv = (values[to] ?? 0) - (values[from] ?? 0);
  return dv < 0 ? Number.NaN : (dv * 1000) / dt;
}

/** Calls `visit` for each rate whose sample lies in (fromMs, toMs], `gap` before a gap. */
function walk(
  columns: TrafficColumns,
  fromMs: number,
  toMs: number,
  visit: (t: number, rx: number, tx: number) => void,
  gap: () => void,
): void {
  const start = firstAfter(columns.at, columns.length, fromMs);
  let base = Math.max(0, start - 1);
  for (let i = Math.max(1, start); i < columns.length; i += 1) {
    const t = columns.at[i] ?? 0;
    if (t > toMs) break;
    const dt = t - (columns.at[base] ?? 0);
    if (dt > TRAFFIC_GAP_MS) {
      gap();
      base = i;
      continue;
    }
    if (dt < MIN_INTERVAL_MS) continue;
    const rx = rate(columns.rx, base, i, dt);
    const tx = rate(columns.tx, base, i, dt);
    base = i;
    visit(t, rx, tx);
  }
}

function fold(max: Float64Array, min: Float64Array, b: number, v: number): void {
  if (Number.isNaN(v)) return;
  const hi = max[b] ?? Number.NaN;
  const lo = min[b] ?? Number.NaN;
  if (Number.isNaN(hi) || v > hi) max[b] = v;
  if (Number.isNaN(lo) || v < lo) min[b] = v;
}

/** Max and min rate per slice of (fromMs, toMs]. */
export function resample(
  columns: TrafficColumns,
  fromMs: number,
  toMs: number,
  count: number,
): Buckets {
  const n = Math.max(1, Math.floor(count));
  const span = toMs - fromMs;
  const at = new Float64Array(n);
  const rxMax = new Float64Array(n).fill(Number.NaN);
  const rxMin = new Float64Array(n).fill(Number.NaN);
  const txMax = new Float64Array(n).fill(Number.NaN);
  const txMin = new Float64Array(n).fill(Number.NaN);
  const breakBefore = new Uint8Array(n);
  for (let b = 0; b < n; b += 1) at[b] = fromMs + ((b + 0.5) * span) / n;
  const out = { from: fromMs, to: toMs, count: n, at, rxMax, rxMin, txMax, txMin, breakBefore };
  if (span <= 0) return out;

  let gap = false;
  walk(
    columns,
    fromMs,
    toMs,
    (t, rx, tx) => {
      if (Number.isNaN(rx) && Number.isNaN(tx)) return;
      // Slices are (start, end]: a rate sits in the slice its sample closes.
      const b = Math.min(n - 1, Math.max(0, Math.ceil(((t - fromMs) * n) / span) - 1));
      // A gap inside one slice is narrower than 2 px; the slice keeps its rate.
      if (gap && Number.isNaN(rxMax[b] ?? Number.NaN) && Number.isNaN(txMax[b] ?? Number.NaN)) {
        breakBefore[b] = 1;
      }
      gap = false;
      at[b] = t;
      fold(rxMax, rxMin, b, rx);
      fold(txMax, txMin, b, tx);
    },
    () => (gap = true),
  );
  return out;
}

/** The largest finite value, or 0. */
export function peak(values: Float64Array): number {
  let top = 0;
  for (const v of values) if (v > top) top = v;
  return top;
}

/** The most recent rate, or null when the last interval is a gap or there is none. */
export function latest(columns: TrafficColumns): Rates | null {
  let last: Rates | null = null;
  const from = (columns.at[Math.max(0, columns.length - 1)] ?? 0) - TRAFFIC_GAP_MS - TICK_MS;
  walk(
    columns,
    from,
    Number.POSITIVE_INFINITY,
    (at, rx, tx) => {
      if (!Number.isNaN(rx) && !Number.isNaN(tx)) last = { at, rx, tx };
    },
    () => (last = null),
  );
  return last;
}

/** Bytes moved in (fromMs, toMs]. Counter resets are dropped; gaps still count. */
export function totals(
  columns: TrafficColumns,
  fromMs: number,
  toMs: number,
): { rx: number; tx: number; samples: number } {
  let rx = 0;
  let tx = 0;
  let samples = 0;
  for (let i = Math.max(1, firstAfter(columns.at, columns.length, fromMs)); i < columns.length; i += 1) {
    if ((columns.at[i] ?? 0) > toMs) break;
    const drx = (columns.rx[i] ?? 0) - (columns.rx[i - 1] ?? 0);
    const dtx = (columns.tx[i] ?? 0) - (columns.tx[i - 1] ?? 0);
    if (drx >= 0) rx += drx;
    if (dtx >= 0) tx += dtx;
    samples += 1;
  }
  return { rx, tx, samples };
}

/** One row of the `Show data` table: bytes in one clock minute, null without samples. */
export type MinuteRow = { readonly start: number; readonly rx: number | null; readonly tx: number | null };

/** Bytes per clock minute over (fromMs, toMs], oldest first. */
export function minutes(columns: TrafficColumns, fromMs: number, toMs: number): MinuteRow[] {
  const MINUTE = RANGE_MS['1m'];
  const rows: MinuteRow[] = [];
  for (let start = Math.floor(fromMs / MINUTE) * MINUTE; start < toMs; start += MINUTE) {
    const sum = totals(columns, Math.max(fromMs, start), Math.min(toMs, start + MINUTE));
    // The window's first minute is often a sliver; empty, it says nothing.
    if (sum.samples === 0 && start < fromMs) continue;
    rows.push({
      start,
      rx: sum.samples > 0 ? sum.rx : null,
      tx: sum.samples > 0 ? sum.tx : null,
    });
  }
  return rows;
}
