import type { Boot, JournalPage, StoredEntry, TrafficSeries } from '@kith/types';
import { AlertTriangle, ArrowRight, Check, ShieldAlert, UserPlus, X } from 'lucide-react';
import { useMemo, useState, type ReactNode } from 'react';
import { api, net } from '../lib/api';
import { useFetch, useLive, useNetwork } from '../lib/data';
import { useRoute } from '../lib/router';
import { ratesFromTail, smooth } from './stats/series';
import { RECENT_MS } from '../components/device/peer';
import { BarList, Button, Empty, fmt, Page, Section, Skeleton, Stat, TimeSeries, toast, type Point } from '../ui';

/** Sums every device's per-second rate into one line; a counter reset is skipped, not counted as zero traffic. */
function throughput(tail: TrafficSeries[], network: string, since: number): { rx: Point[]; tx: Point[] } {
  const total = new Map<number, { t: number; rx: number; tx: number }>();
  for (const d of ratesFromTail(tail, network, since)) {
    d.rx.forEach((pt, i) => {
      const key = Math.round(pt.t / 2000);
      const s = total.get(key) ?? { t: pt.t, rx: 0, tx: 0 };
      s.rx += pt.v;
      s.tx += d.tx[i].v;
      s.t = Math.max(s.t, pt.t);
      total.set(key, s);
    });
  }
  const rows = [...total.values()].sort((x, y) => x.t - y.t);
  return { rx: rows.map((s) => ({ t: s.t, v: s.rx })), tx: rows.map((s) => ({ t: s.t, v: s.tx })) };
}

function exitStat(boot: Boot): { value: string; note?: string } {
  const s = boot.status;
  if (s.exit) return { value: 'Local', note: 'This device is the exit' };
  if (!s.exit_node) return { value: 'None' };
  const value = s.exit_node.slice(s.exit_node.indexOf('/') + 1);
  if (s.exit_state === 'active') return { value, note: 'Internet goes through it' };
  return { value, note: s.exit_state === 'offline_blocked' ? 'Down, internet blocked' : 'Down, going direct' };
}

function reach(b: { port?: number | null; protocol: string }): ReactNode {
  if (b.protocol === 'any') return 'on any protocol';
  const proto = b.protocol.toUpperCase();
  return <>on <span className="whitespace-nowrap">{b.port != null ? `${proto} port ${b.port}` : proto}</span></>;
}

const VERB: Record<string, string> = {
  peer_online: 'came online',
  peer_offline: 'went offline',
  path_changed: 'changed path',
  endpoint_changed: 'moved to a new address',
  device_joined: 'joined',
  device_removed: 'was removed',
  device_renamed: 'was renamed',
  exit_changed: 'changed exit',
  join_requested: 'asked to join',
  blocked: 'was blocked',
  dropped: 'dropped packets',
};

function sentence({ entry }: StoredEntry): string {
  const e = entry as Record<string, unknown> & { kind: string };
  const who = String(e.name ?? e.from_name ?? e.node ?? '');
  if (typeof e.message === 'string') return who ? `${who}: ${e.message}` : e.message;
  const verb = VERB[e.kind] ?? e.kind.replace(/_/g, ' ');
  const extra = e.kind === 'path_changed' ? ` to ${String(e.to ?? 'none')}` : '';
  return `${who} ${verb}${extra}`.trim();
}

function Row({ icon, children, actions }: { icon: ReactNode; children: ReactNode; actions?: ReactNode }) {
  return (
    <li className="flex flex-wrap items-start gap-x-4 gap-y-3 border-b border-line py-4 first:pt-0 last:border-b-0 last:pb-0 sm:flex-nowrap sm:items-center">
      <div className="flex min-w-0 flex-1 basis-full items-start gap-3 sm:basis-0">
        <span className="flex h-5 w-4 shrink-0 items-center justify-center text-fg-2">{icon}</span>
        <div className="min-w-0 flex-1 text-[14px] leading-5 [overflow-wrap:anywhere] [text-wrap:pretty]">{children}</div>
      </div>
      {actions && <div className="flex shrink-0 gap-2 pl-7 sm:pl-0">{actions}</div>}
    </li>
  );
}

export function Overview() {
  const { boot, network, peers } = useNetwork();
  const { refresh, error } = useLive();
  const { go } = useRoute();
  const journal = useFetch<JournalPage>('/api/history/journal', 10000);
  const [busy, setBusy] = useState<string | null>(null);

  const tail = useMemo(
    () => (boot?.traffic_tail ?? []).filter((s) => !network || s.network === network.id),
    [boot?.traffic_tail, network],
  );
  const traffic = useMemo(() => {
    if (!network) return { rx: [], tx: [] };
    const t = throughput(tail, network.id, (boot?.server_time_ms ?? 0) - RECENT_MS);
    return { rx: smooth(t.rx, 10_000), tx: smooth(t.tx, 10_000) };
  }, [tail, network, boot?.server_time_ms]);

  if (!boot && error) {
    return (
      <Page title="Overview" description="The dashboard can't reach kith right now">
        <Empty action={<Button size="sm" variant="secondary" onClick={refresh}>Try again</Button>}>
          {error}
        </Empty>
      </Page>
    );
  }

  if (!boot || !network) {
    return (
      <Page title="Overview" description={<Skeleton width={260} height={14} />}>
        <div className="grid grid-cols-2 gap-x-6 gap-y-8 sm:gap-x-8 lg:grid-cols-4">
          {[0, 1, 2, 3].map((i) => <Skeleton key={i} height={64} />)}
        </div>
        <div className="mt-12"><Skeleton height={240} /></div>
      </Page>
    );
  }

  const online = peers.filter((p) => p.online);
  const direct = online.filter((p) => p.path === 'direct').length;
  const relayed = online.length - direct;
  const { joins, blocked } = boot.pending;
  const problems = network.problems;
  const attention = joins.length + blocked.length + problems.length;
  const health =
    attention === 0
      ? peers.length === 0
        ? 'No devices yet. Add one from Devices to get started'
        : online.length === 0
        ? 'No devices are online right now'
        : online.length === 1
          ? 'The one online device is healthy'
          : `All ${online.length} online devices are healthy`
      : `${attention} thing${attention === 1 ? ' needs' : 's need'} your attention`;

  const fresh = (pts: Point[]) => {
    const last = pts.at(-1);
    return online.length > 0 && last && Date.now() - last.t < 30_000 ? last.v : 0;
  };
  const lastRx = fresh(traffic.rx);
  const lastTx = fresh(traffic.tx);
  const exit = exitStat(boot);

  const act = async (key: string, run: () => Promise<unknown>, done: string, failed: string) => {
    setBusy(key);
    try {
      await run();
      toast(done);
    } catch (e) {
      console.error(e);
      toast(failed);
    } finally {
      setBusy(null);
      refresh();
    }
  };

  const names = new Map(peers.map((p) => [p.node, p.display_name]));
  const busiest = ratesFromTail(tail, network.id, boot.server_time_ms - RECENT_MS)
    .filter((d) => names.has(d.node))
    .map((d) => ({ id: d.node, label: names.get(d.node)!, value: d.total }))
    .filter((p) => p.value > 0)
    .sort((a, b) => b.value - a.value)
    .slice(0, 5);
  const events = (journal.data?.entries ?? []).filter((e) => e.entry.kind !== 'log' && e.entry.kind !== 'dropped').slice(0, 5);

  return (
    <Page title="Overview" description={health}>
      <div className="space-y-12">
        {attention > 0 && (
          <Section title="Needs you">
            <ul>
              {problems.map((p, i) => (
                <Row key={`p${i}`} icon={<AlertTriangle size={16} className="text-hot" />}>
                  <div>{p.what}</div>
                  {p.fix && <div className="mt-0.5 text-[13px] leading-5 text-fg-2">{p.fix}</div>}
                </Row>
              ))}
              {joins.map((j) => (
                <Row
                  key={j.id}
                  icon={<UserPlus size={16} />}
                  actions={
                    <>
                      <Button size="sm" variant="primary" icon={<Check size={14} />} busy={busy === `a${j.id}`} disabled={busy !== null}
                        onClick={() => act(`a${j.id}`, () => api.post(net(j.network, `/joins/${encodeURIComponent(j.node)}/approve`), { name: j.claimed.name }), `${j.claimed.name} approved`, `Could not approve ${j.claimed.name}. Try again.`)}>
                        Approve
                      </Button>
                      <Button size="sm" variant="secondary" icon={<X size={14} />} busy={busy === `d${j.id}`} disabled={busy !== null}
                        onClick={() => act(`d${j.id}`, () => api.post(net(j.network, `/joins/${encodeURIComponent(j.node)}/deny`)), `${j.claimed.name} denied`, `Could not deny ${j.claimed.name}. Try again.`)}>
                        Deny
                      </Button>
                    </>
                  }
                >
                  <div>{j.claimed.name} wants to join</div>
                  <div className="mt-0.5 text-[13px] leading-5 text-fg-2" title={fmt.absolute(j.requested_at_ms)}>
                    Check code <span className="font-mono">{j.check_code}</span> · {j.claimed.os ?? 'Unknown system'} · {fmt.relative(j.requested_at_ms)}
                  </div>
                </Row>
              ))}
              {blocked.map((b) => (
                <Row
                  key={b.id}
                  icon={<ShieldAlert size={16} className="text-hot" />}
                  actions={
                    <Button size="sm" variant="primary" busy={busy === `b${b.id}`} disabled={busy !== null}
                      onClick={() => act(`b${b.id}`, () => api.post(`/api/blocked/${encodeURIComponent(b.id)}/allow`, { preview: false }), `${b.from_name} can now reach ${b.to_name}`, `Could not allow ${b.from_name}. Try again.`)}>
                      Allow
                    </Button>
                  }
                >
                  {b.from_name} tried to reach {b.to_name} {reach(b)}{' '}
                  <span className="whitespace-nowrap text-fg-2">({b.count === 1 ? 'once' : `${b.count} times`})</span>
                </Row>
              ))}
            </ul>
          </Section>
        )}

        <div className="grid grid-cols-2 gap-x-6 gap-y-8 sm:gap-x-8 lg:grid-cols-4">
          <Stat label="Online" value={online.length} unit={peers.length ? `of ${peers.length}` : undefined} />
          <Stat label="Throughput now" value={fmt.rate(lastRx + lastTx)} secondary={<span className="flex flex-wrap gap-x-3"><span className="whitespace-nowrap">↓ {fmt.rate(lastRx)}</span><span className="whitespace-nowrap">↑ {fmt.rate(lastTx)}</span></span>} />
          <Stat label="Exit" value={exit.value} secondary={exit.note} title={exit.value} />
          <Stat label="Paths" value={online.length ? `${direct} / ${relayed}` : '—'} secondary={online.length ? 'direct / relayed' : 'Nothing online to reach'} />
        </div>

        <Section title="Traffic, last 15 minutes">
          <TimeSeries
            format={fmt.rate}
            binary
            height={240}
            series={[
              { id: 'rx', label: 'Received', points: traffic.rx, style: 'area', tone: 0 },
              { id: 'tx', label: 'Sent', points: traffic.tx, tone: 2 },
            ]}
          />
        </Section>

        <div className="grid gap-12 lg:grid-cols-2">
          <Section title="Busiest devices, last 15 minutes">
            {busiest.length ? <BarList items={busiest} format={fmt.bytes} limit={5} /> : <Empty>No traffic in the last 15 minutes.</Empty>}
          </Section>
          <Section
            title="Recent changes"
            actions={<Button size="sm" variant="ghost" icon={<ArrowRight size={14} />} onClick={() => go('/activity')}>All activity</Button>}
          >
            {!journal.data ? (
              journal.error ? <Empty>Could not load recent changes.</Empty> : <Skeleton height={160} />
            ) : events.length === 0 ? (
              <Empty>Nothing has changed yet.</Empty>
            ) : (
              <ul>
                {events.map((e) => (
                  <li key={e.id} className="flex items-baseline gap-3 border-b border-line py-3 text-[14px] leading-5 first:pt-0 last:border-b-0 last:pb-0">
                    <span className="min-w-0 flex-1 truncate" title={sentence(e)}>{sentence(e)}</span>
                    <span className="shrink-0 font-mono text-[12px] tabular-nums text-fg-3" title={fmt.absolute(e.entry.at_ms)}>{fmt.relative(e.entry.at_ms)}</span>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </div>
      </div>
    </Page>
  );
}
