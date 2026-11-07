import { describe, expect, it } from 'vitest';
import { dailyTotals } from './trafficDays';

const day = (d: number, h: number): number => new Date(2026, 10, d, h, 0, 0).getTime();

describe('dailyTotals', () => {
  it('adds up one device by local day, newest first, leaving out quiet days and others', () => {
    const totals = dailyTotals(
      [
        {
          network: 'n',
          node: 'a',
          minutes: [
            { minute_ms: day(5, 1), rx_bytes: 10, tx_bytes: 1 },
            { minute_ms: day(5, 23), rx_bytes: 5, tx_bytes: 2 },
            { minute_ms: day(6, 0), rx_bytes: 0, tx_bytes: 0 },
            { minute_ms: day(7, 12), rx_bytes: 7, tx_bytes: 0 },
          ],
        },
        { network: 'n', node: 'b', minutes: [{ minute_ms: day(7, 12), rx_bytes: 99, tx_bytes: 99 }] },
      ],
      'n',
      'a',
    );
    expect(totals).toEqual([
      { dayMs: new Date(2026, 10, 7).getTime(), rx: 7, tx: 0 },
      { dayMs: new Date(2026, 10, 5).getTime(), rx: 15, tx: 3 },
    ]);
  });
});
