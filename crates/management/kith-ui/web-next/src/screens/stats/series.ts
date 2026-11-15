import type { PathKind, StoredEntry, Tick, TrafficHistory, TrafficSeries } from "@kith/types";
import type { BarItem, Point, Series } from "../../ui";

export type Range = "15m" | "1h" | "6h" | "24h";
export const RANGES: Range[] = ["15m", "1h", "6h", "24h"];
export const RANGE_MS: Record<Range, number> = { "15m": 15 * 60_000, "1h": 3_600_000, "6h": 6 * 3_600_000, "24h": 24 * 3_600_000 };

/** Per-device rates (bytes/s) in the range, with total bytes moved. */
export type DeviceRates = { node: string; rx: Point[]; tx: Point[]; total: number };

/** 2 s cumulative samples → per-second rates; a counter that went backwards (reset) is skipped. */
export function ratesFromTail(tail: TrafficSeries[], network: string, since: number): DeviceRates[] {
  return tail
    .filter((s) => s.network === network)
    .map((s) => {
      const rx: Point[] = [];
      const tx: Point[] = [];
      let total = 0;
      for (let i = 1; i < s.samples.length; i++) {
        const a = s.samples[i - 1];
        const b = s.samples[i];
        const dt = (b.at_ms - a.at_ms) / 1000;
        const dr = b.rx_bytes - a.rx_bytes;
        const dx = b.tx_bytes - a.tx_bytes;
        if (b.at_ms < since || dt <= 0 || dr < 0 || dx < 0) continue;
        rx.push({ t: b.at_ms, v: dr / dt });
        tx.push({ t: b.at_ms, v: dx / dt });
        total += dr + dx;
      }
      return { node: s.node, rx, tx, total };
    });
}

/** Per-minute byte counts → per-second rates, quiet minutes filled with zero. */
export function ratesFromHistory(history: TrafficHistory[], since: number, now: number): DeviceRates[] {
  const start = Math.ceil(since / 60_000) * 60_000;
  const end = Math.floor(now / 60_000) * 60_000;
  return history.map((h) => {
    const byMinute = new Map(h.minutes.map((m) => [m.minute_ms, m]));
    const rx: Point[] = [];
    const tx: Point[] = [];
    let total = 0;
    for (let t = start; t <= end; t += 60_000) {
      const m = byMinute.get(t);
      rx.push({ t, v: (m?.rx_bytes ?? 0) / 60 });
      tx.push({ t, v: (m?.tx_bytes ?? 0) / 60 });
      total += (m?.rx_bytes ?? 0) + (m?.tx_bytes ?? 0);
    }
    return { node: h.node, rx, tx, total };
  });
}

/** Averages a rate into `bucket`-ms windows, so bursty 2 s samples read as a steady rate. */
export function smooth(points: Point[], bucket: number): Point[] {
  const acc = new Map<number, [number, number]>();
  for (const p of points) {
    const t = Math.floor(p.t / bucket) * bucket;
    const a = acc.get(t) ?? [0, 0];
    acc.set(t, [a[0] + p.v, a[1] + 1]);
  }
  return [...acc].sort((a, b) => a[0] - b[0]).map(([t, [sum, n]]) => ({ t, v: sum / n }));
}

/** Each list averaged over the bucket, then summed across lists. */
function sumByTime(lists: Point[][], bucket: number): Point[] {
  const sums = new Map<number, number>();
  for (const list of lists) for (const p of smooth(list, bucket)) sums.set(p.t, (sums.get(p.t) ?? 0) + p.v);
  return [...sums].sort((a, b) => a[0] - b[0]).map(([t, v]) => ({ t, v }));
}

export function totalSeries(devices: DeviceRates[], bucket: number): Series[] {
  return [
    { id: "rx", label: "Received", points: sumByTime(devices.map((d) => d.rx), bucket), style: "area", tone: 0 },
    { id: "tx", label: "Sent", points: sumByTime(devices.map((d) => d.tx), bucket), style: "area", tone: 2 },
  ];
}

// Greys that stay apart on black: a device keeps its own shade, and "Others" is the dimmest.
const TONES = [0, 4, 1, 5, 2] as const;

const UNKNOWN = "Unknown device";
const nameOf = (names: Map<string, string>, node: string) => names.get(node) ?? UNKNOWN;

const LEGEND_MAX = 6;

/** At most six lines: past that, the top five and one "Others" line folded with `fold` at each time. */
function capped(lines: { id: string; label: string; points: Point[] }[], fold: (a: number, b: number) => number, othersLabel: string): Series[] {
  const head = lines.length > LEGEND_MAX ? lines.slice(0, LEGEND_MAX - 1) : lines;
  const out: Series[] = head.map((l, i) => ({ ...l, tone: TONES[i % TONES.length] }));
  if (lines.length > LEGEND_MAX) {
    const acc = new Map<number, number>();
    for (const l of lines.slice(LEGEND_MAX - 1)) for (const p of l.points) acc.set(p.t, acc.has(p.t) ? fold(acc.get(p.t)!, p.v) : p.v);
    out.push({ id: "others", label: othersLabel, points: [...acc].sort((a, b) => a[0] - b[0]).map(([t, v]) => ({ t, v })), tone: 3 });
  }
  return out;
}

/** Each active device as one rx+tx line, busiest first; the rest summed as "Others". */
export function deviceSeries(devices: DeviceRates[], names: Map<string, string>, bucket: number): Series[] {
  const lines = [...devices]
    .filter((d) => d.total > 0)
    .sort((a, b) => b.total - a.total)
    .map((d) => ({
      id: d.node,
      label: nameOf(names, d.node),
      points: smooth(d.rx.map((p, j) => ({ t: p.t, v: p.v + (d.tx[j]?.v ?? 0) })), bucket),
    }));
  return capped(lines, (a, b) => a + b, `${Math.max(0, lines.length - (LEGEND_MAX - 1))} other devices`);
}

export function busiest(devices: DeviceRates[], names: Map<string, string>): BarItem[] {
  return devices.filter((d) => d.total > 0).map((d) => ({ id: d.node, label: nameOf(names, d.node), value: d.total }));
}

export const lastValue = (s: Series | undefined) => s?.points.at(-1)?.v ?? 0;

const linkKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`);

/** One line per link (either direction), measured `pick` from the ticks in range, worst mean first; the rest folded into "Others" (their worst). */
export function linkSeries(
  ticks: readonly Tick[],
  network: string,
  since: number,
  names: Map<string, string>,
  pick: (l: Tick["links"][number]) => number | null,
): Series[] {
  const lines = new Map<string, { label: string; points: Point[] }>();
  for (const tick of ticks) {
    if (tick.at_ms < since) continue;
    for (const l of tick.links) {
      if (l.network !== network) continue;
      const v = pick(l);
      if (v == null) continue;
      const key = linkKey(l.from, l.to);
      let line = lines.get(key);
      if (!line) {
        line = { label: `${nameOf(names, l.from)} ↔ ${nameOf(names, l.to)}`, points: [] };
        lines.set(key, line);
      }
      const last = line.points.at(-1);
      if (last?.t === tick.at_ms) last.v = Math.max(last.v, v);
      else line.points.push({ t: tick.at_ms, v });
    }
  }
  const mean = (ps: Point[]) => ps.reduce((s, p) => s + p.v, 0) / (ps.length || 1);
  const sorted = [...lines].sort((a, b) => mean(b[1].points) - mean(a[1].points)).map(([id, l]) => ({ id, ...l }));
  return capped(sorted, Math.max, `Worst of ${Math.max(0, sorted.length - (LEGEND_MAX - 1))} other links`);
}

const PATHS: { kind: PathKind; label: string }[] = [
  { kind: "direct", label: "Direct" },
  { kind: "relay", label: "Relayed" },
  { kind: "stealth", label: "Stealth" },
];

/** Links per path kind at each tick, a link counted once whichever end reported it. */
export function pathSeries(ticks: readonly Tick[], network: string, since: number): Series[] {
  const out = PATHS.map((p, i) => ({ id: p.kind, label: p.label, points: [] as Point[], tone: TONES[i] }));
  for (const tick of ticks) {
    if (tick.at_ms < since) continue;
    const seen = new Map<string, PathKind>();
    for (const l of tick.links) if (l.network === network) seen.set(linkKey(l.from, l.to), l.path);
    const counts = new Map<PathKind, number>();
    for (const k of seen.values()) counts.set(k, (counts.get(k) ?? 0) + 1);
    for (const s of out) s.points.push({ t: tick.at_ms, v: counts.get(s.id) ?? 0 });
  }
  return out;
}

/** Devices online as a step line, walked back from `onlineNow` through the journal (newest first); starts where the journal does when it was cut short inside the range. */
export function onlineSeries(entries: StoredEntry[], complete: boolean, network: string, onlineNow: number, since: number, now: number): { points: Point[]; from: number } {
  const points: Point[] = [{ t: now, v: onlineNow }];
  let count = onlineNow;
  for (const { entry } of entries) {
    if (entry.at_ms < since) break;
    if (entry.kind !== "peer_online" && entry.kind !== "peer_offline") continue;
    if (entry.network !== network) continue;
    points.push({ t: entry.at_ms, v: count });
    count = Math.max(0, count + (entry.kind === "peer_online" ? -1 : 1));
    points.push({ t: entry.at_ms - 1, v: count });
  }
  const oldest = entries.at(-1)?.entry.at_ms ?? now;
  const from = complete ? since : Math.max(since, Math.min(oldest, now));
  if (from < points[points.length - 1].t) points.push({ t: from, v: count });
  return { points: points.reverse(), from };
}

export function dropItems(drops: Record<string, number>): BarItem[] {
  return Object.entries(drops)
    .filter(([, n]) => n > 0)
    .map(([reason, n]) => ({ id: reason, label: reason.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase()), value: n }));
}

/** The oldest minute with any traffic in the history, or null. */
export function oldestTraffic(history: TrafficHistory[]): number | null {
  let oldest: number | null = null;
  for (const h of history) for (const m of h.minutes) if (m.rx_bytes + m.tx_bytes > 0 && (oldest === null || m.minute_ms < oldest)) oldest = m.minute_ms;
  return oldest;
}
