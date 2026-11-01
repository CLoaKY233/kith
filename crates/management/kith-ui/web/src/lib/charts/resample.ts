// Traffic counters to rates, and max-bucket resampling (spec §9.1). Workstream F owns it.
// Rate = Δbytes/Δt between samples; negative deltas are dropped; gaps over TRAFFIC_GAP_MS
// break the line (NaN).

import { TRAFFIC_GAP_MS } from '../live/timing';
import type { TrafficColumns } from '../types';

/** Bytes per second between consecutive samples; NaN marks a gap or a counter reset. */
export function rates(columns: TrafficColumns, which: 'rx' | 'tx'): Float64Array {
  const out = new Float64Array(Math.max(0, columns.length - 1));
  const values = columns[which];
  for (let i = 1; i < columns.length; i += 1) {
    const dt = (columns.at[i] ?? 0) - (columns.at[i - 1] ?? 0);
    const dv = (values[i] ?? 0) - (values[i - 1] ?? 0);
    out[i - 1] = dt <= 0 || dt > TRAFFIC_GAP_MS || dv < 0 ? Number.NaN : (dv * 1000) / dt;
  }
  return out;
}

/** The maximum of each of `buckets` equal slices; NaN when a slice holds only gaps. */
export function maxBuckets(values: Float64Array, buckets: number): Float64Array {
  const out = new Float64Array(Math.max(0, buckets)).fill(Number.NaN);
  if (values.length === 0 || buckets <= 0) return out;
  for (let i = 0; i < values.length; i += 1) {
    const v = values[i] ?? Number.NaN;
    if (Number.isNaN(v)) continue;
    const b = Math.min(buckets - 1, Math.floor((i * buckets) / values.length));
    const current = out[b] ?? Number.NaN;
    if (Number.isNaN(current) || v > current) out[b] = v;
  }
  return out;
}
