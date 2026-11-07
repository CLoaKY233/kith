// A device's traffic per day, from what the Console kept (`GET /api/history/traffic`:
// bytes per minute, 30 days). The chart covers the last hour; this reaches further back.

import type { TrafficHistory } from '../types.gen';
import { startOfDay } from './activity';

export type DayTotal = { readonly dayMs: number; readonly rx: number; readonly tx: number };

/** Bytes per day in the viewer's time zone, newest first; quiet days are left out. */
export function dailyTotals(history: readonly TrafficHistory[], network: string, node: string): DayTotal[] {
  const days = new Map<number, { rx: number; tx: number }>();
  for (const series of history) {
    if (series.network !== network || series.node !== node) continue;
    for (const minute of series.minutes) {
      const key = startOfDay(minute.minute_ms);
      const total = days.get(key) ?? { rx: 0, tx: 0 };
      total.rx += minute.rx_bytes;
      total.tx += minute.tx_bytes;
      days.set(key, total);
    }
  }
  return [...days]
    .filter(([, total]) => total.rx > 0 || total.tx > 0)
    .sort(([a], [b]) => b - a)
    .map(([dayMs, total]) => ({ dayMs, ...total }));
}
