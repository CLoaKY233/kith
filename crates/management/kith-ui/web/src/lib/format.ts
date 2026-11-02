// Numbers and times as the page shows them (spec §5 Rate, Rtt, RelativeTime, CheckCode;
// §3.3 units; §6.5 day dividers). Units sit after a thin space; bytes are SI (1000) with
// 3 significant figures. Times use one fixed 24-hour English format so they read like the
// rest of the copy: `14:02`, `Tue 14:02`, `Mon 31 Oct`.

import { copy, THIN } from './copy';
import {
  IDLE_BYTES_PER_SEC,
  MINUTE,
  RELATIVE_NOW_MS,
  RELATIVE_TIME_CLOCK_AFTER_MS,
  RELATIVE_TIME_DATE_AFTER_MS,
  RELATIVE_TIME_WEEKDAY_AFTER_MS,
  SECOND,
} from './live/timing';

/** A number and its unit, for markup that sets the unit in `--ink-3` (§3.3). */
export type Parts = { readonly value: string; readonly unit: string };

// The page's copy is English, so dates are too; en-GB gives `Mon 31 Oct`, and h23 avoids
// the `24:05` some engines print for hour12:false.
const LOCALE = 'en-GB';
const hm = new Intl.DateTimeFormat(LOCALE, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
const hms = new Intl.DateTimeFormat(LOCALE, {
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hourCycle: 'h23',
});
const weekday = new Intl.DateTimeFormat(LOCALE, { weekday: 'short' });
const dayMonth = new Intl.DateTimeFormat(LOCALE, { day: 'numeric', month: 'short' });
const weekdayDayMonth = new Intl.DateTimeFormat(LOCALE, {
  weekday: 'short',
  day: 'numeric',
  month: 'short',
});

const join = (parts: Parts): string => (parts.unit ? `${parts.value}${THIN}${parts.unit}` : parts.value);

/** SI scaling to 3 significant figures; rounding that reaches 1000 moves up a unit. */
function si(amount: number, units: readonly string[]): Parts {
  let n = Number.isFinite(amount) ? Math.max(0, amount) : 0;
  let unit = 0;
  while (n >= 1000 && unit < units.length - 1) {
    n /= 1000;
    unit += 1;
  }
  let rounded = unit === 0 ? Math.round(n) : Number(n.toPrecision(3));
  if (rounded >= 1000 && unit < units.length - 1) {
    unit += 1;
    rounded = Number((rounded / 1000).toPrecision(3));
  }
  const digits = unit === 0 || rounded >= 100 ? 0 : rounded >= 10 ? 1 : 2;
  return { value: rounded.toFixed(digits), unit: units[unit] ?? '' };
}

/** §5 Rate as parts: `824` `kB/s`; `idle` below 1 kB/s; `—` when unknown. */
export function rateParts(bytesPerSec: number | null): Parts {
  if (bytesPerSec === null || !Number.isFinite(bytesPerSec)) return { value: copy.units.none, unit: '' };
  if (bytesPerSec < IDLE_BYTES_PER_SEC) return { value: copy.units.idle, unit: '' };
  return si(bytesPerSec, copy.units.bytesPerSec);
}

/** `824 kB/s`, `1.24 MB/s`, `idle` below 1 kB/s, `—` when unknown. */
export function rate(bytesPerSec: number | null): string {
  return join(rateParts(bytesPerSec));
}

/** Totals as parts: `412` `MB`. */
export function bytesParts(total: number): Parts {
  return si(total, copy.units.bytes);
}

/** `412 MB`. */
export function bytes(total: number): string {
  return join(bytesParts(total));
}

/** §5 Rtt as parts: `12` `ms`, or `—`. Anything above zero shows at least 1 ms. */
export function rttParts(ms: number | null): Parts {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return { value: copy.units.none, unit: '' };
  const whole = ms > 0 ? Math.max(1, Math.round(ms)) : 0;
  return { value: String(whole), unit: copy.units.ms };
}

/** `12 ms`, or `—`. */
export function rtt(ms: number | null): string {
  return join(rttParts(ms));
}

/** `14:02`. */
export function clock(ms: number): string {
  return hm.format(ms);
}

/** `14:02:16`, for chart readouts and tooltips. */
export function clockSeconds(ms: number): string {
  return hms.format(ms);
}

/** `Tue 14:02`. */
export function weekdayClock(ms: number): string {
  return `${weekday.format(ms)} ${clock(ms)}`;
}

/** `Mon 31 Oct`. */
export function day(ms: number): string {
  return weekdayDayMonth.format(ms);
}

/** `12 Oct`. */
export function shortDate(ms: number): string {
  return dayMonth.format(ms);
}

/** `Mon 31 Oct 14:02:16`: the absolute time for a `title` attribute. */
export function absolute(ms: number): string {
  return `${day(ms)} ${clockSeconds(ms)}`;
}

/** The `datetime` attribute of a `<time>`. */
export function isoTime(ms: number): string {
  return new Date(ms).toISOString();
}

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/**
 * §5 RelativeTime: `now`, `12 s ago`, `4 min ago`, then `14:02` after 1 h on the same
 * day, then `Tue 14:02`. Beyond a week a weekday alone is ambiguous, so it reads
 * `12 Oct 14:02`. Pass server time as `nowMs` (`live.now()`); future times read `now`.
 */
export function relative(atMs: number, nowMs: number): string {
  const age = Math.max(0, nowMs - atMs);
  if (age < RELATIVE_NOW_MS) return copy.time.now;
  if (age < MINUTE) return copy.time.secondsAgo(Math.floor(age / SECOND));
  if (age < RELATIVE_TIME_CLOCK_AFTER_MS) return copy.time.minutesAgo(Math.floor(age / MINUTE));
  if (age < RELATIVE_TIME_WEEKDAY_AFTER_MS && startOfDay(atMs) === startOfDay(nowMs)) {
    return clock(atMs);
  }
  if (age < RELATIVE_TIME_DATE_AFTER_MS) return weekdayClock(atMs);
  return `${shortDate(atMs)} ${clock(atMs)}`;
}

/** A future moment, as precise as needed: `14:02` today, `Tue 14:02` within a week,
 * `Mon 31 Oct 14:02` beyond. */
export function until(atMs: number, nowMs: number): string {
  if (startOfDay(atMs) === startOfDay(nowMs)) return clock(atMs);
  if (atMs - nowMs < RELATIVE_TIME_DATE_AFTER_MS) return weekdayClock(atMs);
  return `${day(atMs)} ${clock(atMs)}`;
}

/** §6.5 day dividers: `Today`, `Yesterday`, `Mon 31 Oct`. */
export function dayLabel(atMs: number, nowMs: number): string {
  const today = startOfDay(nowMs);
  const that = startOfDay(atMs);
  if (that === today) return copy.time.today;
  if (that === startOfDay(today - 1)) return copy.time.yesterday;
  return day(atMs);
}

/** `3 h ago`, for `seen …` on offline devices (`PeerInfo.last_seen_secs`). */
export function ago(seconds: number): string {
  const s = Math.max(0, seconds);
  if (s * SECOND < RELATIVE_NOW_MS) return copy.time.now;
  if (s < 60) return copy.time.secondsAgo(Math.floor(s));
  if (s < 3600) return copy.time.minutesAgo(Math.floor(s / 60));
  if (s < 86_400) return copy.time.hoursAgo(Math.floor(s / 3600));
  return copy.time.daysAgo(Math.floor(s / 86_400));
}

/** `K7Q4XM` or `k7q-4xm` → `K7Q·4XM` (§2.10, §5 CheckCode). */
export function checkCode(code: string): string {
  const clean = code.replace(/[^0-9A-Za-z]/g, '').toUpperCase();
  return clean.length === 6 ? `${clean.slice(0, 3)}·${clean.slice(3)}` : clean;
}
