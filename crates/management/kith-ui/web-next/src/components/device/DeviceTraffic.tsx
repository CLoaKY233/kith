import { useEffect, useMemo, useRef, useState } from 'react';
import type { TrafficHistory } from '@kith/types';
import { SegmentedControl, SegmentedControlIndicator, SegmentedControlItem } from '@ui-sdk/react/segmented-control';
import { useFetch } from '../../lib/data';
import { Panel, Skeleton, TimeSeries, fmt, type Series } from '../../ui';

const RANGES = { '1h': 3600_000, '6h': 6 * 3600_000, '24h': 24 * 3600_000 } as const;
type Range = keyof typeof RANGES;
const MINUTE = 60_000;
const HOUR = 3600_000;

/** Bytes per second per minute from the Console's history; quiet minutes count as zero. */
export function DeviceTraffic({ network, node, span }: { network: string; node: string; span?: 8 | 12 }) {
  const [range, setRange] = useState<Range>('1h');
  const [minute, setMinute] = useState(() => Math.floor(Date.now() / MINUTE));
  useEffect(() => {
    const timer = setInterval(() => setMinute(Math.floor(Date.now() / MINUTE)), 15_000);
    return () => clearInterval(timer);
  }, []);
  // Ask from a whole hour back so the request only changes hourly; the chart trims to the range.
  const ask = Math.floor((minute * MINUTE - RANGES[range]) / HOUR) * HOUR;
  const { data: fresh, error } = useFetch<TrafficHistory[]>(
    `/api/history/traffic?${new URLSearchParams({ network, node, since_ms: String(ask) })}`,
    60_000,
  );
  const kept = useRef<{ key: string; data: TrafficHistory[] } | null>(null);
  const key = `${network}|${node}`;
  useEffect(() => {
    if (fresh) kept.current = { key, data: fresh };
  }, [fresh, key]);
  const data = fresh ?? (kept.current?.key === key ? kept.current.data : null);
  const series = useMemo<Series[]>(() => {
    const minutes = data?.find((h) => h.node === node)?.minutes ?? [];
    if (!minutes.length) return [];
    const by = new Map(minutes.map((m) => [m.minute_ms, m]));
    const start = Math.floor((minute * MINUTE - RANGES[range]) / MINUTE) * MINUTE;
    // The minute still running is partial, so it would read as a dip.
    const end = (minute - 1) * MINUTE;
    const rx = [];
    const tx = [];
    for (let t = start; t <= end; t += MINUTE) {
      const m = by.get(t);
      rx.push({ t, v: (m?.rx_bytes ?? 0) / 60 });
      tx.push({ t, v: (m?.tx_bytes ?? 0) / 60 });
    }
    return [
      { id: 'rx', label: 'Received', points: rx, style: 'area', tone: 0 },
      { id: 'tx', label: 'Sent', points: tx, style: 'line', tone: 2 },
    ];
  }, [data, node, minute, range]);

  return (
    <Panel
      span={span}
      label="Traffic"
      right={
        <SegmentedControl aria-label="Time range" value={range} onValueChange={(v) => v && setRange(v as Range)}>
          <SegmentedControlIndicator />
          {(Object.keys(RANGES) as Range[]).map((r) => (
            <SegmentedControlItem key={r} value={r}>{r}</SegmentedControlItem>
          ))}
        </SegmentedControl>
      }
    >
      {!data && !error ? (
        <Skeleton height={220} />
      ) : error && !data ? (
        <div className="grid h-[220px] place-items-center text-[13px] text-fg-3">Traffic history is not available right now.</div>
      ) : (
        <TimeSeries series={series} format={fmt.rate} binary height={220} sync={false} />
      )}
    </Panel>
  );
}
