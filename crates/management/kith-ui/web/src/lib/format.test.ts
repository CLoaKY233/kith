import { describe, expect, it } from 'vitest';
import { THIN } from './copy';
import { ago, bytes, checkCode, clock, dayLabel, rate, rateParts, relative, rtt } from './format';

const at = (y: number, mo: number, d: number, h: number, mi: number, s = 0): number =>
  new Date(y, mo - 1, d, h, mi, s).getTime();
const u = (value: string, unit: string): string => `${value}${THIN}${unit}`;

describe('rates and sizes (spec §5 Rate, §9.1)', () => {
  it('uses SI bytes with three significant figures and carries rounding into the next unit', () => {
    expect(rate(824_000)).toBe(u('824', 'kB/s'));
    expect(rate(1_240_000)).toBe(u('1.24', 'MB/s'));
    expect(rate(12_440_000)).toBe(u('12.4', 'MB/s'));
    expect(rate(999_600)).toBe(u('1.00', 'MB/s'));
    expect(rate(99_960)).toBe(u('100', 'kB/s'));
    expect(bytes(412_000_000)).toBe(u('412', 'MB'));
    expect(bytes(999)).toBe(u('999', 'B'));
  });

  it('reads idle below 1 kB/s and a dash when unknown', () => {
    expect(rate(999)).toBe('idle');
    expect(rate(null)).toBe('—');
    expect(rate(Number.NaN)).toBe('—');
    expect(rateParts(1_000)).toEqual({ value: '1.00', unit: 'kB/s' });
  });

  it('shows latency in whole milliseconds, never 0 for a real round trip', () => {
    expect(rtt(12.4)).toBe(u('12', 'ms'));
    expect(rtt(0.3)).toBe(u('1', 'ms'));
    expect(rtt(null)).toBe('—');
  });
});

describe('times (spec §5 RelativeTime, §6.5)', () => {
  const now = at(2026, 11, 3, 15, 30);

  it('steps from now through seconds and minutes to a clock, a weekday and a date', () => {
    expect(relative(now - 2_000, now)).toBe('now');
    expect(relative(now + 5_000, now)).toBe('now');
    expect(relative(now - 12_500, now)).toBe(u('12', 's ago'));
    expect(relative(now - 4 * 60_000 - 59_000, now)).toBe(u('4', 'min ago'));
    expect(relative(at(2026, 11, 3, 14, 2), now)).toBe('14:02');
    expect(relative(at(2026, 11, 2, 23, 50), now)).toBe('Mon 23:50');
    expect(relative(at(2026, 10, 20, 9, 5), now)).toBe('20 Oct 09:05');
  });

  it('writes 24-hour clocks without the 24:xx quirk', () => {
    expect(clock(at(2026, 11, 3, 0, 5))).toBe('00:05');
  });

  it('labels Activity days as Today, Yesterday, then the date', () => {
    expect(dayLabel(at(2026, 11, 3, 0, 1), now)).toBe('Today');
    expect(dayLabel(at(2026, 11, 2, 23, 59), now)).toBe('Yesterday');
    expect(dayLabel(at(2026, 10, 31, 12, 0), now)).toBe('Sat 31 Oct');
  });

  it('writes ages for seen-ago text', () => {
    expect(ago(3)).toBe('now');
    expect(ago(3 * 3600 + 5)).toBe(u('3', 'h ago'));
    expect(ago(3 * 86_400)).toBe('3 days ago');
  });
});

describe('check codes (spec §2.10)', () => {
  it('groups six characters as K7Q·4XM whatever the input spacing or case', () => {
    expect(checkCode('K7Q4XM')).toBe('K7Q·4XM');
    expect(checkCode('k7q-4xm')).toBe('K7Q·4XM');
    expect(checkCode('K7Q·4XM')).toBe('K7Q·4XM');
  });
});
