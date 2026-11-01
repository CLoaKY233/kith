// Numbers and times as the page shows them (spec §5 Rate, Rtt, RelativeTime, CheckCode).
// Units sit after a thin space; bytes are SI (1000) with 3 significant figures.

import { copy, THIN } from './copy';
import {
  IDLE_BYTES_PER_SEC,
  RELATIVE_NOW_MS,
  RELATIVE_TIME_CLOCK_AFTER_MS,
  RELATIVE_TIME_WEEKDAY_AFTER_MS,
} from './live/timing';

function si(value: number, units: readonly string[]): string {
  let n = Math.max(0, value);
  let unit = 0;
  while (n >= 1000 && unit < units.length - 1) {
    n /= 1000;
    unit += 1;
  }
  const digits = unit === 0 ? 0 : n >= 100 ? 0 : n >= 10 ? 1 : 2;
  return `${n.toFixed(digits)}${THIN}${units[unit] ?? ''}`;
}

/** `824 kB/s`, `1.24 MB/s`, `idle` below 1 kB/s, `—` when unknown. */
export function rate(bytesPerSec: number | null): string {
  if (bytesPerSec === null || !Number.isFinite(bytesPerSec)) return copy.units.none;
  if (bytesPerSec < IDLE_BYTES_PER_SEC) return copy.units.idle;
  return si(bytesPerSec, copy.units.bytesPerSec);
}

/** `412 MB`. */
export function bytes(total: number): string {
  return si(total, copy.units.bytes);
}

/** `12 ms`, or `—`. */
export function rtt(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms)) return copy.units.none;
  return `${Math.round(ms)}${THIN}${copy.units.ms}`;
}

/** `14:02` in the viewer's locale, 24-hour. */
export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
}

/** `14:02:16`. */
export function clockSeconds(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

/** `Tue 14:02`. */
export function weekdayClock(ms: number): string {
  const day = new Date(ms).toLocaleDateString(undefined, { weekday: 'short' });
  return `${day} ${clock(ms)}`;
}

/** `Mon 31 Oct`, for Activity day dividers. */
export function day(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
  });
}

/** `12 Oct`. */
export function shortDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}

/** §5 RelativeTime: `now`, `12 s ago`, `4 min ago`, `14:02`, `Tue 14:02`. */
export function relative(atMs: number, nowMs: number): string {
  const age = Math.max(0, nowMs - atMs);
  if (age < RELATIVE_NOW_MS) return copy.time.now;
  if (age < 60_000) return copy.time.secondsAgo(Math.floor(age / 1000));
  if (age < RELATIVE_TIME_CLOCK_AFTER_MS) return copy.time.minutesAgo(Math.floor(age / 60_000));
  if (age < RELATIVE_TIME_WEEKDAY_AFTER_MS) return clock(atMs);
  return weekdayClock(atMs);
}

/** `3 h ago` style ages for `seen …` (offline devices). */
export function ago(seconds: number): string {
  if (seconds * 1000 < RELATIVE_NOW_MS) return copy.time.now;
  if (seconds < 60) return copy.time.secondsAgo(Math.floor(seconds));
  if (seconds < 3600) return copy.time.minutesAgo(Math.floor(seconds / 60));
  if (seconds < 86_400) return copy.time.hoursAgo(Math.floor(seconds / 3600));
  return copy.time.daysAgo(Math.floor(seconds / 86_400));
}

/** `K7Q4XM` → `K7Q·4XM`. */
export function checkCode(code: string): string {
  const clean = code.replace(/[^0-9A-Za-z]/g, '').toUpperCase();
  return clean.length === 6 ? `${clean.slice(0, 3)}·${clean.slice(3)}` : clean;
}
