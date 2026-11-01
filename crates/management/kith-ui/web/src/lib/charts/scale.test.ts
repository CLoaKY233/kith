import { describe, expect, it } from 'vitest';
import { THIN } from '../copy';
import { Y_SHRINK_AFTER_MS } from '../live/timing';
import { pathAt, pathSpans } from './events';
import { rangeFor } from './range';
import { axisRate, GrowShrink, sparkFit, ticks } from './scale';
import type { Entry } from '../types.gen';

describe('spark scale', () => {
  it('refits only when the peak leaves 0.5–1.25 of the fit', () => {
    expect(sparkFit(0, 40_000)).toBe(40_000);
    expect(sparkFit(40_000, 49_000)).toBe(40_000);
    expect(sparkFit(40_000, 51_000)).toBe(51_000);
    expect(sparkFit(40_000, 21_000)).toBe(40_000);
    expect(sparkFit(40_000, 19_000)).toBe(19_000);
  });

  it('keeps idle rows flat instead of scaling keep-alives to full height', () => {
    expect(sparkFit(0, 30)).toBe(1000);
  });
});

describe('detail scale', () => {
  it('grows at once and shrinks only after 10 s of 3× headroom', () => {
    const s = new GrowShrink();
    expect(s.update(100_000, 0)).toEqual({ top: 125_000, animate: false });
    expect(s.update(200_000, 1)).toEqual({ top: 250_000, animate: false });
    expect(s.update(50_000, 2).top).toBe(250_000);
    expect(s.update(50_000, 2 + Y_SHRINK_AFTER_MS - 1).top).toBe(250_000);
    expect(s.update(50_000, 2 + Y_SHRINK_AFTER_MS)).toEqual({ top: 62_500, animate: true });
  });

  it('forgets the headroom clock when traffic comes back', () => {
    const s = new GrowShrink();
    s.update(300_000, 0);
    s.update(10_000, 1);
    s.update(200_000, 2);
    expect(s.update(10_000, 3 + Y_SHRINK_AFTER_MS - 2).animate).toBe(false);
  });
});

describe('axis', () => {
  it('puts at most 3 ticks of one 1-2-5 step on both sides, none at 0', () => {
    expect(ticks(12_400_000, 4_100_000)).toEqual([5_000_000, 10_000_000]);
    expect(ticks(3_000_000, 3_000_000)).toEqual([2_000_000, -2_000_000]);
    expect(ticks(1250, 416)).toEqual([500, 1000]);
    for (const [up, down] of [
      [9e6, 3e6],
      [1e3, 1e3],
      [7.7e8, 2.6e8],
    ] as const) {
      expect(ticks(up, down).length).toBeLessThanOrEqual(3);
      expect(ticks(up, down)).not.toContain(0);
    }
  });

  it('labels received above and sent below', () => {
    expect(axisRate(10_000_000)).toBe(`↓ 10${THIN}MB/s`);
    expect(axisRate(-1_500_000)).toBe(`↑ 1.5${THIN}MB/s`);
    expect(axisRate(500)).toBe(`↓ 500${THIN}B/s`);
  });
});

describe('path history', () => {
  const entry = (seq: number, at_ms: number, rest: object): Entry =>
    ({ seq, at_ms, ...rest }) as Entry;
  const base = { network: 'home', node: 'nas', name: 'nas' };

  it('rebuilds relayed stretches from path entries, in seq order', () => {
    const events = [
      entry(3, 3000, { kind: 'path_changed', ...base, from: 'relay', to: 'direct' }),
      entry(1, 1000, { kind: 'path_changed', ...base, from: 'direct', to: 'relay' }),
      entry(2, 2000, { kind: 'peer_online', ...base, node: 'tv', path: 'relay', gateway: null }),
    ];
    const spans = pathSpans(events, 'nas', 'direct', 0, 5000);
    expect(spans).toEqual([
      { from: 0, to: 1000, path: 'direct' },
      { from: 1000, to: 3000, path: 'relay' },
      { from: 3000, to: 5000, path: 'direct' },
    ]);
    expect(pathAt(spans, 2500)).toBe('relay');
  });

  it('shows no path while offline, and the live path without history', () => {
    const events = [entry(1, 1000, { kind: 'peer_offline', ...base })];
    expect(pathSpans(events, 'nas', 'direct', 0, 2000)).toEqual([
      { from: 0, to: 1000, path: null },
      { from: 1000, to: 2000, path: null },
    ]);
    expect(pathSpans([], 'nas', 'stealth', 0, 2000)).toEqual([{ from: 0, to: 2000, path: 'stealth' }]);
  });
});

describe('range for a linked moment', () => {
  it('picks the smallest range that shows the moment with room to its left', () => {
    expect(rangeFor(100_000 - 20_000, 100_000)).toBe('1m');
    expect(rangeFor(1_000_000 - 58_000, 1_000_000)).toBe('15m');
    expect(rangeFor(10_000_000 - 20 * 60_000, 10_000_000)).toBe('1h');
    expect(rangeFor(0, 10_000_000)).toBe('1h');
  });
});
