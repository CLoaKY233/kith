// Y-scales for the traffic charts, and the labels on the detail chart's axis.
//
// Spark (§5): each row keeps the peak it was fitted to and draws up to SPARK_RESCALE_UP
// times that. It refits only when the window's peak leaves [DOWN, UP] times the fit, so a
// row's line does not breathe with every sample.
//
// Series (§9.1): the top grows the moment a peak passes it, and shrinks only after
// Y_SHRINK_AFTER_MS with more than Y_SHRINK_HEADROOM times headroom.

import { copy, THIN } from '../copy';
import {
  IDLE_BYTES_PER_SEC,
  SPARK_RESCALE_DOWN,
  SPARK_RESCALE_UP,
  Y_SHRINK_AFTER_MS,
  Y_SHRINK_HEADROOM,
} from '../live/timing';

/** The smallest fit: below 1 kB/s a row reads idle, so its line stays near the floor. */
const FLOOR = IDLE_BYTES_PER_SEC;

/** The fit a spark should use for a window whose peak is `top`. */
export function sparkFit(current: number, top: number): number {
  const wanted = Math.max(FLOOR, top);
  if (current <= 0) return wanted;
  if (wanted > current * SPARK_RESCALE_UP || wanted < current * SPARK_RESCALE_DOWN) return wanted;
  return current;
}

/** The value drawn at a spark's top edge for a fit. */
export function sparkTop(fit: number): number {
  return fit * SPARK_RESCALE_UP;
}

/** What the detail chart's scale should do after a new peak. */
export type ScaleStep = { readonly top: number; readonly animate: boolean };

/** The detail chart's top for one direction (§9.1). Feed it every frame's peak. */
export class GrowShrink {
  private top = 0;
  private roomySince: number | null = null;

  /** Forgets the scale, for a new device or range. */
  reset(): void {
    this.top = 0;
    this.roomySince = null;
  }

  update(top: number, now: number): ScaleStep {
    const wanted = Math.max(FLOOR, top) * SPARK_RESCALE_UP;
    if (this.top === 0 || top > this.top) {
      this.top = wanted;
      this.roomySince = null;
      return { top: this.top, animate: false };
    }
    if (this.top > Y_SHRINK_HEADROOM * Math.max(FLOOR, top)) {
      this.roomySince ??= now;
      if (now - this.roomySince >= Y_SHRINK_AFTER_MS) {
        this.top = wanted;
        this.roomySince = null;
        return { top: this.top, animate: true };
      }
    } else {
      this.roomySince = null;
    }
    return { top: this.top, animate: false };
  }
}

/**
 * Up to `max` ticks of one 1-2-5 step, above and below the baseline, never at 0. Values
 * below the baseline are negative: the chart draws sent traffic downwards.
 */
export function ticks(upper: number, lower: number, max = 3): number[] {
  if (!(upper > 0) && !(lower > 0)) return [];
  const span = Math.max(upper, lower);
  let step = 10 ** Math.floor(Math.log10(span / max));
  for (let guard = 0; guard < 64; guard += 1) {
    const count = Math.floor(upper / step) + Math.floor(lower / step);
    if (count <= max) break;
    const digit = step / 10 ** Math.floor(Math.log10(step) + 1e-9);
    step *= digit < 1.5 ? 2 : digit < 3 ? 2.5 : 2;
  }
  const out: number[] = [];
  for (let v = step; v <= upper + 1e-9; v += step) out.push(v);
  for (let v = step; v <= lower + 1e-9; v += step) out.push(-v);
  return out;
}

/** An axis label: `↓ 10 MB/s` above the baseline, `↑ 5 MB/s` below. */
export function axisRate(value: number): string {
  const units = copy.units.bytesPerSec;
  let n = Math.abs(value);
  let unit = 0;
  while (n >= 1000 && unit < units.length - 1) {
    n /= 1000;
    unit += 1;
  }
  const arrow = value < 0 ? copy.units.up : copy.units.down;
  return `${arrow} ${Number(n.toPrecision(3))}${THIN}${units[unit] ?? ''}`;
}
