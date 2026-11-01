// Easing for canvas motion. Charts cannot use CSS transitions, so they evaluate the same
// `cubic-bezier()` strings that tokens.css uses (EASING in timing.ts), and a value that
// eases from one number to another over a duration from timing.ts.

type Curve = (t: number) => number;

const linear: Curve = (t) => t;

/** Solves a CSS `cubic-bezier(x1, y1, x2, y2)` for y at progress t in [0, 1]. */
function bezier(x1: number, y1: number, x2: number, y2: number): Curve {
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  const x = (s: number): number => ((ax * s + bx) * s + cx) * s;
  const dx = (s: number): number => (3 * ax * s + 2 * bx) * s + cx;
  const y = (s: number): number => ((ay * s + by) * s + cy) * s;
  return (t) => {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    let s = t;
    for (let i = 0; i < 8; i += 1) {
      const err = x(s) - t;
      if (Math.abs(err) < 1e-6) return y(s);
      const slope = dx(s);
      if (Math.abs(slope) < 1e-6) break;
      s -= err / slope;
    }
    let lo = 0;
    let hi = 1;
    s = t;
    for (let i = 0; i < 32; i += 1) {
      if (x(s) < t) lo = s;
      else hi = s;
      s = (lo + hi) / 2;
    }
    return y(s);
  };
}

/** Parses an EASING value (`linear` or `cubic-bezier(...)`); anything else is linear. */
export function curve(css: string): Curve {
  const match = /^cubic-bezier\(([^)]+)\)$/.exec(css.trim());
  if (!match?.[1]) return linear;
  const [x1, y1, x2, y2] = match[1].split(',').map((v) => Number.parseFloat(v));
  if ([x1, y1, x2, y2].some((v) => v === undefined || !Number.isFinite(v))) return linear;
  return bezier(x1 ?? 0, y1 ?? 0, x2 ?? 1, y2 ?? 1);
}

/**
 * A number that eases towards its target. `set` starts a new ease from wherever the value
 * is now, so a retarget mid-flight never jumps.
 */
export class Tween {
  private from: number;
  private to: number;
  private start = 0;

  constructor(
    value: number,
    private readonly duration: number,
    private readonly ease: Curve,
  ) {
    this.from = value;
    this.to = value;
  }

  get target(): number {
    return this.to;
  }

  /** Eases to `value` from `now`; `instant` jumps there (reduced motion, first frame). */
  set(value: number, now: number, instant = false): void {
    if (value === this.to && !instant) return;
    this.from = instant ? value : this.value(now);
    this.to = value;
    this.start = now;
  }

  value(now: number): number {
    if (this.duration <= 0) return this.to;
    const t = Math.min(1, Math.max(0, (now - this.start) / this.duration));
    return this.from + (this.to - this.from) * this.ease(t);
  }

  /** True while the value is still moving at `now`. */
  moving(now: number): boolean {
    return this.from !== this.to && now - this.start < this.duration;
  }
}
