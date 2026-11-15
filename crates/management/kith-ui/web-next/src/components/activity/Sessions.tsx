import { useSyncExternalStore } from 'react';
import type { Entry } from '@kith/types';
import {
  Waterfall, WaterfallAxis, WaterfallBar, WaterfallCell, WaterfallColumnHeader,
  WaterfallHeaderRow, WaterfallIndicator, WaterfallRow, WaterfallRowHeader, WaterfallTick,
  type WaterfallData,
} from '@ui-sdk/react/waterfall';
import { ChevronRight } from 'lucide-react';
import { fmt } from '../../ui';

type Session = { start: number; end: number; open: boolean; unknown?: boolean };
type Device = { node: string; name: string; list: Session[] };

/**
 * Each device's online sessions, from peer_online / peer_offline entries. A device that never went
 * offline in the window and is online now was online since before the window, so its bar starts there.
 */
export function sessionsOf(entries: Entry[], now: number, peers: { node: string; name: string; online: boolean }[] | null) {
  const sorted = [...entries].sort((a, b) => a.at_ms - b.at_ms);
  const t0 = sorted[0]?.at_ms ?? now;
  const open = new Map<string, number | null>();
  const byNode = new Map<string, Device>();
  for (const e of sorted) {
    if (e.kind !== 'peer_online' && e.kind !== 'peer_offline') continue;
    const d = byNode.get(e.node) ?? { node: e.node, name: e.name, list: [] };
    d.name = e.name;
    const since = open.get(e.node) ?? null;
    if (e.kind === 'peer_online') {
      if (since == null) open.set(e.node, e.at_ms);
    } else {
      d.list.push({ start: since ?? t0, end: e.at_ms, open: false, unknown: since == null });
      open.set(e.node, null);
    }
    byNode.set(e.node, d);
  }
  const onlineNow = new Set(peers?.filter((p) => p.online).map((p) => p.node));
  for (const p of peers ?? []) {
    if (p.online && !byNode.has(p.node)) byNode.set(p.node, { node: p.node, name: p.name, list: [] });
  }
  for (const d of byNode.values()) {
    const since = open.get(d.node);
    // A "came online" in the window is a known start; with no event at all, the device was
    // already online when the window began.
    if (since != null) d.list.push({ start: since, end: now, open: true });
    else if (d.list.length === 0 && onlineNow.has(d.node)) d.list.push({ start: t0, end: now, open: true, unknown: true });
  }
  return { t0, devices: [...byNode.values()].filter((d) => d.list.length) };
}

const dur = (ms: number) => {
  const m = Math.round(ms / 60000);
  if (m < 1) return '<1m';
  if (m < 60) return `${m}m`;
  return m % 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m / 60}h`;
};
const span = (s: Session) => {
  if (!s.unknown) return `${fmt.clock(s.start)}–${s.open ? 'now' : fmt.clock(s.end)}`;
  return s.open ? 'Online since before this window' : `Online until ${fmt.clock(s.end)}, since before this window`;
};
const length = (ms: number, unknown: boolean) => (unknown ? (ms < 60_000 ? 'Online' : `>${dur(ms)}`) : dur(ms));
const NARROW = '(max-width: 639px)';
const watchNarrow = (cb: () => void) => {
  const m = window.matchMedia(NARROW);
  m.addEventListener('change', cb);
  return () => m.removeEventListener('change', cb);
};
/** Axis ticks on whole local hours, as offsets from t0, thinned to at most `max`. */
function hourTicks(t0: number, now: number, max: number) {
  const span = now - t0;
  const step = [1, 2, 3, 4, 6, 12, 24].find((h) => Math.ceil(span / 3_600_000 / h) <= max) ?? 24;
  const d = new Date(t0);
  d.setMinutes(0, 0, 0);
  const out: { time: number; label: string }[] = [];
  for (let t = d.getTime(); t < now; t += 3_600_000) {
    if (new Date(t).getHours() % step !== 0) continue;
    const at = (t - t0) / span;
    if (at >= 0.06 && at <= 0.94) out.push({ time: t - t0, label: fmt.clock(t) });
  }
  return out;
}
const tone = (s: Session) => (s.unknown ? 'bg-fg/40' : s.open ? 'bg-fg' : 'bg-fg-3');

/** Online sessions on one time axis; expand a device to see each session. */
export function Sessions({ t0, devices, now }: ReturnType<typeof sessionsOf> & { now: number }) {
  const items: WaterfallData = devices.map((d) => {
    const first = d.list[0].start;
    return {
      value: d.node,
      label: d.name,
      start: first - t0,
      duration: Math.max(1, d.list[d.list.length - 1].end - first),
      children: d.list.map((s, i) => ({
        value: `${d.node}#${i}`,
        label: `${d.name}, session ${i + 1}`,
        start: s.start - t0,
        duration: Math.max(1, s.end - s.start),
      })),
    };
  });
  const rows = devices.flatMap((d) => [
    { value: d.node, d, s: null as Session | null, i: 0 },
    ...d.list.map((s, i) => ({ value: `${d.node}#${i}`, d, s, i })),
  ]);
  const narrow = useSyncExternalStore(watchNarrow, () => window.matchMedia(NARROW).matches, () => false);
  const online = new Map<string, string>();
  for (const d of devices) {
    online.set(d.node, length(d.list.reduce((a, x) => a + x.end - x.start, 0), d.list.some((x) => x.unknown)));
    d.list.forEach((s, i) => online.set(`${d.node}#${i}`, length(s.end - s.start, !!s.unknown)));
  }
  const grid = 'grid grid-cols-[minmax(0,96px)_72px_1fr] items-center gap-3 sm:grid-cols-[minmax(0,140px)_80px_1fr] sm:gap-4';
  return (
    <Waterfall
      aria-label="Who was online, when"
      items={items}
      format={(ms) => fmt.clock(t0 + ms)}
      getBarLabel={(item) => `${item.label}, online ${online.get(String(item.value))?.replace('>', 'over ') ?? dur(item.duration)}`}
      className="isolate text-[13px]"
    >
      <WaterfallHeaderRow className={`${grid} pb-2`}>
        <WaterfallColumnHeader className="pl-[18px] text-left font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">Device</WaterfallColumnHeader>
        <WaterfallColumnHeader className="text-right font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">Online</WaterfallColumnHeader>
        <WaterfallColumnHeader aria-label="Time" className="relative h-5">
          <WaterfallAxis className="absolute inset-0 font-mono text-[11px] leading-5 text-fg-3 [&>*]:absolute [&>*]:left-[var(--waterfall-tick)] [&>*]:-translate-x-1/2">
            {() => hourTicks(t0, now, narrow ? 2 : 5).map((t) => <WaterfallTick key={t.time} time={t.time}>{t.label}</WaterfallTick>)}
          </WaterfallAxis>
        </WaterfallColumnHeader>
      </WaterfallHeaderRow>
      <div role="rowgroup" className="max-h-80 overflow-y-auto overflow-x-hidden">
        {rows.map(({ value, d, s, i }) => {
          const total = s ? s.end - s.start : d.list.reduce((a, x) => a + x.end - x.start, 0);
          const first = d.list[0].start;
          const width = Math.max(1, d.list[d.list.length - 1].end - first);
          return (
            <WaterfallRow key={value} value={value} className={`${grid} h-8 outline-none hover:bg-ink-2 focus-visible:bg-ink-2`}>
              <WaterfallRowHeader className="flex min-w-0 items-center gap-1 pl-[calc((var(--waterfall-level)-1)*18px)]">
                {!s && (
                  <WaterfallIndicator className="grid h-4 w-3.5 shrink-0 place-items-center text-fg-3 transition-transform data-[expanded]:rotate-90">
                    <ChevronRight size={14} />
                  </WaterfallIndicator>
                )}
                <span title={s ? undefined : d.name} className={`truncate ${s ? 'font-mono text-[11px] text-fg-3' : 'text-fg'}`}>{s ? `Session ${i + 1}` : d.name}</span>
              </WaterfallRowHeader>
              <WaterfallCell className="whitespace-nowrap text-right font-mono text-[12px] tabular-nums text-fg">
                {length(total, s ? !!s.unknown : d.list.some((x) => x.unknown))}
              </WaterfallCell>
              <WaterfallBar className="relative h-full">
                <div className="absolute top-1/2 h-2 min-w-[4px] -translate-y-1/2 left-[min(var(--waterfall-start),calc(100%-4px))] w-[var(--waterfall-size)]">
                  {s ? (
                    <div title={span(s)} className={`h-full min-w-[4px] ${tone(s)}`} />
                  ) : (
                    <div className="relative h-full bg-ink-2">
                      {d.list.map((x, k) => (
                        <div
                          key={k}
                          title={span(x)}
                          className={`absolute inset-y-0 min-w-[4px] ${tone(x)}`}
                          style={{ left: `${((x.start - first) / width) * 100}%`, width: `${((x.end - x.start) / width) * 100}%` }}
                        />
                      ))}
                    </div>
                  )}
                </div>
              </WaterfallBar>
            </WaterfallRow>
          );
        })}
      </div>
    </Waterfall>
  );
}
