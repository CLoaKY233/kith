// Per-device traffic counters as columns (spec §8.2): one Float64Array each for time,
// received and sent bytes, oldest first, at most TRAFFIC_SAMPLES (1 h at one per tick).
// Rates are Δbytes/Δt between the last two samples; a counter that went backwards (the
// device's session restarted) gives no rate (§9.1).

import type { TrafficColumns } from '../types';
import { TICK_MS, TRAFFIC_SAMPLES } from './timing';

/** Two samples closer than this are the same moment read twice (the boot snapshot's tail
 * and its last tick); the later one replaces the earlier instead of adding a point. */
const SAME_SAMPLE_MS = TICK_MS / 2;

export class Columns implements TrafficColumns {
  readonly at: Float64Array;
  readonly rx: Float64Array;
  readonly tx: Float64Array;
  length = 0;

  constructor(readonly capacity: number = TRAFFIC_SAMPLES) {
    this.at = new Float64Array(capacity);
    this.rx = new Float64Array(capacity);
    this.tx = new Float64Array(capacity);
  }

  /** Adds a sample. Returns false when it is older than the last one and was ignored. */
  record(at: number, rx: number, tx: number): boolean {
    const last = this.length - 1;
    if (last >= 0) {
      const lastAt = this.at[last] ?? 0;
      if (at < lastAt) return false;
      if (at - lastAt < SAME_SAMPLE_MS) {
        this.at[last] = at;
        this.rx[last] = rx;
        this.tx[last] = tx;
        return true;
      }
    }
    if (this.length === this.capacity) {
      this.at.copyWithin(0, 1);
      this.rx.copyWithin(0, 1);
      this.tx.copyWithin(0, 1);
      this.length -= 1;
    }
    this.at[this.length] = at;
    this.rx[this.length] = rx;
    this.tx[this.length] = tx;
    this.length += 1;
    return true;
  }

  /** Bytes per second over the last two samples; `null` without two usable samples. */
  rates(): { rx: number; tx: number } | null {
    if (this.length < 2) return null;
    const i = this.length - 1;
    const dt = ((this.at[i] ?? 0) - (this.at[i - 1] ?? 0)) / 1000;
    const drx = (this.rx[i] ?? 0) - (this.rx[i - 1] ?? 0);
    const dtx = (this.tx[i] ?? 0) - (this.tx[i - 1] ?? 0);
    if (dt <= 0 || drx < 0 || dtx < 0) return null;
    return { rx: drx / dt, tx: dtx / dt };
  }

  /** Whether received bytes grew over the last two samples: something arrived. */
  heard(): boolean {
    if (this.length < 2) return false;
    const i = this.length - 1;
    return (this.rx[i] ?? 0) > (this.rx[i - 1] ?? 0);
  }
}

/** `older` samples (oldest first) in front of `current`'s, keeping the newest capacity. */
export function merged(older: readonly { at_ms: number; rx_bytes: number; tx_bytes: number }[], current: Columns): Columns {
  const out = new Columns(current.capacity);
  const first = current.length > 0 ? (current.at[0] ?? 0) : Number.POSITIVE_INFINITY;
  for (const sample of older) {
    if (sample.at_ms <= first - SAME_SAMPLE_MS) out.record(sample.at_ms, sample.rx_bytes, sample.tx_bytes);
  }
  for (let i = 0; i < current.length; i += 1) out.record(current.at[i] ?? 0, current.rx[i] ?? 0, current.tx[i] ?? 0);
  return out;
}
