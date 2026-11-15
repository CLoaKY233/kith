import { useMemo, useRef, useState, type ReactNode } from "react";
import { SegmentedControl, SegmentedControlIndicator, SegmentedControlItem } from "@ui-sdk/react/segmented-control";
import type { JournalPage, TrafficHistory } from "@kith/types";
import { BarList, Empty, Page, Panel, PanelGrid, Skeleton, StatusDot, TimeSeries, fmt, type Series } from "../ui";
import { useFetch, useLive, useNetwork } from "../lib/data";
import {
  RANGES,
  RANGE_MS,
  busiest,
  deviceSeries,
  dropItems,
  lastValue,
  oldestTraffic,
  linkSeries,
  onlineSeries,
  pathSeries,
  ratesFromHistory,
  ratesFromTail,
  totalSeries,
  type Range,
} from "./stats/series";

const H = 200;

export function Stats() {
  const [range, setRange] = useState<Range>("1h");
  const { error, ticks } = useLive();
  const { boot, network, peers } = useNetwork();
  const now = boot?.server_time_ms ?? 0;
  const since = now - RANGE_MS[range];
  // Minute-resolution history only needs recomputing once a minute, not on every 2 s poll.
  const rateNow = range === "15m" ? now : Math.floor(now / 60_000) * 60_000;
  const rateSince = rateNow - RANGE_MS[range];
  const bucket = range === "15m" ? 10_000 : 60_000;
  const net = network?.id ?? "";

  // The whole day is fetched once (rounded to the minute so the query only changes once a minute);
  // each range is a slice of it, and its extent says which ranges have anything to show.
  // Rounded to the hour, so the path (which resets the fetched data) rarely changes; the last good history is kept across it.
  const dayStart = Math.floor((now - RANGE_MS["24h"]) / 3_600_000) * 3_600_000;
  const fetched = useFetch<TrafficHistory[]>(
    net && now ? `/api/history/traffic?${new URLSearchParams({ network: net, since_ms: String(dayStart) })}` : null,
    60_000,
  );
  const kept = useRef<TrafficHistory[] | null>(null);
  if (fetched.data) kept.current = fetched.data;
  const history = { data: fetched.data ?? kept.current, error: fetched.error };
  const journal = useFetch<JournalPage>(net ? `/api/history/journal?${new URLSearchParams({ network: net, limit: "1000" })}` : null, 30_000);

  // Keyed by content, so the 2 s poll doesn't hand every chart below a new map.
  const self = boot?.status.node;
  const selfName = boot?.status.memberships.find((x) => x.network === net)?.name ?? boot?.status.host.name ?? "";
  const nameKey = JSON.stringify([...peers.map((p) => [p.node, p.name]), ...(self ? [[self, selfName]] : [])]);
  const names = useMemo(() => new Map<string, string>(JSON.parse(nameKey) as [string, string][]), [nameKey]);

  const devices = useMemo(() => {
    if (!boot || !net) return null;
    if (range === "15m") return ratesFromTail(boot.traffic_tail, net, rateSince);
    return history.data ? ratesFromHistory(history.data, rateSince, rateNow) : null;
  }, [range === "15m" ? boot : null, net, range, rateSince, rateNow, history.data]);

  const total = useMemo(() => (devices ? totalSeries(devices, bucket) : []), [devices, bucket]);
  const byDevice = useMemo(() => (devices ? deviceSeries(devices, names, bucket) : []), [devices, names, bucket]);
  const top = useMemo(() => (devices ? busiest(devices, names) : []), [devices, names]);
  const latency = useMemo(() => linkSeries(ticks, net, since, names, (l) => l.rtt_ms), [ticks, net, since, names]);
  const loss = useMemo(() => linkSeries(ticks, net, since, names, (l) => (l.loss == null ? null : l.loss * 100)), [ticks, net, since, names]);
  const paths = useMemo(() => pathSeries(ticks, net, since), [ticks, net, since]);
  const onlineNow = peers.filter((p) => p.online).length;
  const online = useMemo(
    () => (journal.data ? onlineSeries(journal.data.entries, journal.data.next_before === null, net, onlineNow, since, now) : { points: [], from: since }),
    [journal.data, net, onlineNow, since, now],
  );
  const bootDrops = boot?.status.drops;
  const drops = useMemo(() => (bootDrops ? dropItems(bootDrops) : []), [bootDrops]);
  const onlineSeriesList = useMemo(() => [{ id: "online", label: "Online", points: online.points, style: "area" as const }], [online.points]);

  const buffered = ticks[0]?.at_ms ?? now;
  const ticksNote = buffered > since + 10_000 ? "Since you opened this page" : undefined;
  const latestLinks = ticks.at(-1)?.links.filter((l) => l.network === net) ?? [];
  // A link is reported from both ends: count it once, at its worse reading.
  const worstByLink = (pick: (l: (typeof latestLinks)[number]) => number | null) => {
    const m = new Map<string, number>();
    for (const l of latestLinks) {
      const v = pick(l);
      if (v == null) continue;
      const key = l.from < l.to ? `${l.from}|${l.to}` : `${l.to}|${l.from}`;
      m.set(key, Math.max(m.get(key) ?? v, v));
    }
    return [...m.values()];
  };
  const rtts = worstByLink((l) => l.rtt_ms).sort((a, b) => a - b);
  const losses = worstByLink((l) => l.loss);
  const pathNow = paths.map((s) => `${lastValue(s)} ${s.label.toLowerCase()}`).filter((t) => !t.startsWith("0 ")).join(" · ");
  const totalBytes = top.reduce((s, b) => s + b.value, 0);
  const trafficError = range !== "15m" && !history.data ? history.error : null;

  const oldest = history.data ? oldestTraffic(history.data) : null;
  // A range is pointless when the history doesn't reach past the next shorter one: it would show the same.
  const emptyWhy = (r: Range): string | null => {
    const i = RANGES.indexOf(r);
    if (i === 0 || !history.data) return null;
    const shorter = RANGES[i - 1];
    if (oldest === null) return `No traffic recorded yet, so ${r} would be empty`;
    if (oldest >= now - RANGE_MS[shorter]) return `Traffic history starts ${fmt.relative(oldest, now)}, so ${r} would look the same as ${shorter}`;
    return null;
  };

  const live = error ? "Not updating" : boot ? "Live" : "Connecting";
  const actions = (
    <>
      <span className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3" title={error ?? "Refreshing every 2 s"}>
        <StatusDot status={error ? "problem" : boot ? "online" : "offline"} label={live} />
        {live}
      </span>
      <SegmentedControl value={range} onValueChange={(v) => v && setRange(v as Range)} aria-label="Range">
        <SegmentedControlIndicator />
        {RANGES.map((r) => {
          const why = r === range ? null : emptyWhy(r);
          return <SegmentedControlItem key={r} value={r} disabled={!!why} title={why ?? undefined}>{r}</SegmentedControlItem>;
        })}
      </SegmentedControl>
    </>
  );

  if (!boot) {
    return (
      <Page title="Stats" description="How the network is performing over time." actions={actions}>
        {error ? (
          <Empty>Couldn't load the network. {error}</Empty>
        ) : (
          <PanelGrid>
            {([[12, "Total throughput"], [12, "Throughput by device"], [6, "Latency by link"], [6, "Packet loss by link"], [6, "Connection paths"], [6, "Devices online"]] as const).map(([span, label]) => (
              <Panel key={label} span={span} label={label}>
                <Skeleton height={H} />
              </Panel>
            ))}
          </PanelGrid>
        )}
      </Page>
    );
  }

  // A history range's newest minute is still filling, so its rate reads low: quote the one before.
  const current = (s: Series | undefined) => (range === "15m" || !s || s.points.length < 2 ? lastValue(s) : s.points[s.points.length - 2].v);
  const traffic = (body: ReactNode) =>
    trafficError ? <Empty>Couldn't load the traffic history. {trafficError}</Empty> : devices ? body : <Skeleton height={H} />;
  const note = (text?: string) => (text ? <p className="mt-3 text-[12px] text-fg-3">{text}</p> : null);
  const plural = (n: number, one: string, many = `${one}s`) => `${fmt.count(n)} ${n === 1 ? one : many}`;
  const value = (v: ReactNode) => <span className="block max-w-[20ch] truncate whitespace-nowrap text-right tabular-nums" title={typeof v === "string" ? v : undefined}>{v}</span>;

  return (
    <Page title="Stats" description="How the network is performing over time." actions={actions}>
      <PanelGrid>
        <Panel span={12} label="Total throughput" right={value(devices ? fmt.rate(current(total[0]) + current(total[1])) : "—")}>
          {traffic(<TimeSeries series={total} format={fmt.rate} binary height={H} reserveLegend />)}
        </Panel>
        <Panel span={12} label="Throughput by device" right={value(devices ? `${fmt.count(devices.filter((d) => d.total > 0).length)} with traffic` : "—")}>
          {traffic(<TimeSeries series={byDevice} format={fmt.rate} binary height={H} />)}
        </Panel>
        <Panel span={6} label="Latency by link" right={value(rtts.length ? `${fmt.ms(rtts[Math.floor(rtts.length / 2)])} median` : "—")}>
          <TimeSeries series={latency} format={fmt.ms} height={H} reserveLegend />{note(ticksNote)}
        </Panel>
        <Panel span={6} label="Packet loss by link" right={value(losses.length ? `${fmt.percent(Math.max(...losses) * 100)} worst` : "—")}>
          <TimeSeries series={loss} format={fmt.percent} height={H} reserveLegend />{note(ticksNote)}
        </Panel>
        <Panel span={6} label="Connection paths" right={value(pathNow || "—")}>
          <TimeSeries series={paths} format={fmt.count} height={H} integer reserveLegend />{note(ticksNote)}
        </Panel>
        <Panel span={6} label="Devices online" right={value(peers.length ? `${fmt.count(onlineNow)} of ${plural(peers.length, "device")}` : "No devices")}>
          {journal.error && !journal.data ? (
            <Empty>Couldn't load the device history. {journal.error}</Empty>
          ) : journal.data ? (
            <>
              <TimeSeries series={onlineSeriesList} format={fmt.count} height={H} integer reserveLegend />
              {note(online.from > since ? `History starts ${fmt.relative(online.from, now)}` : undefined)}
            </>
          ) : (
            <Skeleton height={H} />
          )}
        </Panel>
        <Panel span={drops.length ? 6 : 12} label="Busiest devices" right={value(devices ? `${fmt.bytes(totalBytes)} total` : "—")}>
          {traffic(top.length ? <BarList items={top} format={fmt.bytes} limit={8} /> : <Empty>No traffic in this range.</Empty>)}
        </Panel>
        {drops.length > 0 && (
          <Panel span={6} label="Drops by reason" right={value(plural(drops.reduce((s, d) => s + d.value, 0), "packet"))}>
            <BarList items={drops} format={fmt.count} limit={8} />
            {note("Since kith started")}
          </Panel>
        )}
      </PanelGrid>
    </Page>
  );
}
