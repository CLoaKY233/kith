import { useState } from 'react';
import type { Entry } from '@kith/types';
import { fmt } from '../../ui';

const HOUR = 3_600_000;

/** Events per hour over the last 24 hours, darker for busier hours; hover or tap a cell for its count. */
export function Density({ entries, now }: { entries: Entry[]; now: number }) {
  const [hover, setHover] = useState<number | null>(null);
  // Whole local hours, so cell edges read as clock times in any timezone.
  const top = new Date(now);
  top.setMinutes(0, 0, 0);
  const end = top.getTime() + HOUR;
  const start = end - 24 * HOUR;
  const counts = Array<number>(24).fill(0);
  for (const e of entries) if (e.at_ms >= start && e.at_ms < end) counts[Math.floor((e.at_ms - start) / HOUR)]++;
  const max = Math.max(1, ...counts);
  const total = counts.reduce((a, n) => a + n, 0);
  const label = (h: number) =>
    `${fmt.clock(start + h * HOUR)}–${fmt.clock(start + (h + 1) * HOUR)}: ${counts[h]} event${counts[h] === 1 ? '' : 's'}`;
  const busiest = counts.indexOf(Math.max(...counts));
  const summary = total ? `Events per hour, last 24 hours. Busiest: ${label(busiest)}.` : 'No events in the last 24 hours.';
  return (
    <div onPointerLeave={(e) => e.pointerType === 'mouse' && setHover(null)}>
      <div role="img" aria-label={summary} className="grid grid-cols-24 gap-px">
        {counts.map((n, h) => (
          <div
            key={h}
            onPointerEnter={() => setHover(h)}
            onPointerDown={() => setHover(h)}
            className={`h-8 bg-ink-2 ${hover === h ? 'outline outline-1 outline-fg' : ''}`}
            style={n ? { background: 'var(--color-fg)', opacity: 0.15 + 0.85 * (n / max) } : undefined}
          />
        ))}
      </div>
      <div className="relative mt-2 h-4 font-mono text-[11px] tabular-nums text-fg-3" aria-hidden="true">
        {hover === null ? (
          <>
            <span className="absolute left-0">{fmt.clock(start)}</span>
            <span className="absolute left-1/2 -translate-x-1/2">{fmt.clock(start + 12 * HOUR)}</span>
            <span className="absolute right-0">Now</span>
          </>
        ) : (
          <span className="absolute left-1/2 max-w-full -translate-x-1/2 truncate whitespace-nowrap text-fg">{label(hover)}</span>
        )}
      </div>
    </div>
  );
}
