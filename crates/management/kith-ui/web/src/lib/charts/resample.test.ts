import { describe, expect, it } from 'vitest';
import { TICK_MS, TRAFFIC_GAP_MS } from '../live/timing';
import type { TrafficColumns } from '../types';
import { bucketCount, latest, minutes, resample, totals } from './resample';

/** Counters from [at, rx, tx] rows. */
function cols(rows: [number, number, number][]): TrafficColumns {
  return {
    at: Float64Array.from(rows, (r) => r[0]),
    rx: Float64Array.from(rows, (r) => r[1]),
    tx: Float64Array.from(rows, (r) => r[2]),
    length: rows.length,
  };
}

/** A steady stream: `rate` bytes per second both ways, one sample per tick. */
function steady(from: number, count: number, rate: number, start = 0): [number, number, number][] {
  return Array.from({ length: count }, (_, i) => {
    const bytes = start + (rate * i * TICK_MS) / 1000;
    return [from + i * TICK_MS, bytes, bytes / 2];
  });
}

const values = (a: Float64Array): (number | null)[] => Array.from(a, (v) => (Number.isNaN(v) ? null : v));

describe('resample', () => {
  it('keeps the peak of each slice and the trough beside it', () => {
    // 2 s ticks: 1000, 5000, 2000, 2000 B/s, two intervals per 4 s slice.
    const c = cols([
      [0, 0, 0],
      [2000, 2000, 0],
      [4000, 12000, 0],
      [6000, 16000, 0],
      [8000, 20000, 0],
    ]);
    const b = resample(c, 0, 8000, 2);
    expect(values(b.rxMax)).toEqual([5000, 2000]);
    expect(values(b.rxMin)).toEqual([1000, 2000]);
    expect(Array.from(b.at)).toEqual([4000, 8000]);
    expect(Array.from(b.breakBefore)).toEqual([0, 0]);
  });

  it('breaks the line at a gap, and never across it', () => {
    const before = steady(0, 5, 1000);
    const after = steady(8000 + TRAFFIC_GAP_MS + 1, 5, 1000, 9000);
    const b = resample(cols([...before, ...after]), 0, 30_000, 15);
    const filled = values(b.rxMax).map((v) => v !== null);
    const firstAfter = filled.indexOf(true, filled.indexOf(false, 1));
    expect(b.breakBefore[firstAfter]).toBe(1);
    expect(Array.from(b.breakBefore).filter(Boolean)).toHaveLength(1);
    // The interval that spans the gap gives no rate at all.
    expect(values(b.rxMax).filter((v) => v !== null && v !== 1000)).toEqual([]);
  });

  it('drops a counter reset without breaking the line', () => {
    const c = cols([
      [0, 0, 0],
      [2000, 4000, 2000],
      [4000, 100, 2000],
      [6000, 2100, 4000],
    ]);
    const b = resample(c, 0, 6000, 3);
    expect(values(b.rxMax)).toEqual([2000, null, 1000]);
    expect(values(b.txMax)).toEqual([1000, 0, 1000]);
    expect(Array.from(b.breakBefore)).toEqual([0, 0, 0]);
  });

  it('spreads a sample taken moments after the last over the next interval', () => {
    const c = cols([
      [0, 0, 0],
      [2000, 2000, 0],
      [2280, 3000, 0],
      [4000, 6000, 0],
    ]);
    const b = resample(c, 0, 4000, 2);
    expect(values(b.rxMax)).toEqual([1000, 2000]);
  });

  it('places slices by time, so missing samples do not shift the axis', () => {
    const c = cols([
      [0, 0, 0],
      [2000, 2000, 0],
      // 4000 is missing; 4 s is under the gap limit, so the line carries on.
      [6000, 6000, 0],
    ]);
    const b = resample(c, 0, 6000, 3);
    expect(values(b.rxMax)).toEqual([1000, null, 1000]);
    expect(Array.from(b.breakBefore)).toEqual([0, 0, 0]);
  });

  it('never makes slices thinner than a sample, nor more than one per 2 px', () => {
    expect(bucketCount(120, 300_000)).toBe(60);
    expect(bucketCount(800, 60_000)).toBe(60_000 / TICK_MS);
    expect(bucketCount(0, 60_000)).toBe(1);
  });
});

describe('totals and latest', () => {
  const c = cols([
    [0, 0, 0],
    [2000, 1000, 500],
    [4000, 200, 700],
    [4000 + TRAFFIC_GAP_MS + 1, 1200, 900],
  ]);

  it('counts bytes across gaps but not across resets', () => {
    expect(totals(c, 0, 20_000)).toEqual({ rx: 2000, tx: 900, samples: 3 });
  });

  it('reports no latest rate once the last interval is a gap', () => {
    expect(latest(c)).toBeNull();
    expect(latest(cols(steady(0, 3, 4000)))).toEqual({ at: 4000, rx: 4000, tx: 2000 });
  });

  it('splits into clock minutes and marks minutes without samples', () => {
    const rows = minutes(cols(steady(0, 31, 1000)), 0, 120_000);
    expect(rows.map((r) => r.rx)).toEqual([60_000, null]);
    // A sliver of a minute before the first sample is left out, not shown as missing.
    expect(minutes(cols(steady(60_000, 31, 1000)), 59_000, 120_000).map((r) => r.rx)).toEqual([60_000]);
  });
});
