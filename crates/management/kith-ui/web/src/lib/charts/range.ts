// The range shared by every chart on a screen (§5 Series): `?range=1m|15m|1h` in the URL.

import { RANGE_MS } from '../live/timing';
import type { Range } from '../types';

export const RANGES: readonly Range[] = ['1m', '15m', '1h'];
export const DEFAULT_RANGE: Range = '1h';

/** The range in `query`, or the default when it is missing or unknown. */
export function rangeFrom(query: URLSearchParams): Range {
  const value = query.get('range');
  return RANGES.find((r) => r === value) ?? DEFAULT_RANGE;
}

/**
 * The smallest range that shows a moment `atMs` with a tenth of the range to spare on its
 * left, for links that park the cursor there (§6.5). Older moments get the widest range.
 */
export function rangeFor(atMs: number, nowMs: number): Range {
  const age = Math.max(0, nowMs - atMs);
  return RANGES.find((r) => age <= RANGE_MS[r] * 0.9) ?? '1h';
}
