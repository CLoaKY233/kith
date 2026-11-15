import { useMemo, useState } from 'react';
import type { PeerInfo } from '@kith/types';
import { Input } from '@ui-sdk/react/input';
import { SegmentedControl, SegmentedControlIndicator, SegmentedControlItem } from '@ui-sdk/react/segmented-control';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@ui-sdk/react/select';
import { CloudOffIcon, MonitorSmartphoneIcon, PlusIcon } from 'lucide-react';
import { useLive, useNetwork } from '../lib/data';
import { useRoute } from '../lib/router';
import { Badge, Button, Empty, Page, Skeleton, Spark, StatusDot, fmt } from '../ui';
import { DevicesActions } from '../components/device/DevicesActions';
import { isPhone, isRelayed, linkFor, liveRates, pathText, seen, sameName } from '../components/device/peer';

type Filter = 'all' | 'online' | 'offline' | 'relayed' | 'phones';
type Sort = 'status' | 'name' | 'traffic' | 'seen';

const FILTERS: { value: Filter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'online', label: 'Online' },
  { value: 'offline', label: 'Offline' },
  { value: 'relayed', label: 'Relayed' },
  { value: 'phones', label: 'Phones' },
];
const SORTS: { value: Sort; label: string }[] = [
  { value: 'status', label: 'Status' },
  { value: 'name', label: 'Name' },
  { value: 'traffic', label: 'Traffic' },
  { value: 'seen', label: 'Last seen' },
];
const ORDER: Record<Sort, (a: PeerInfo, b: PeerInfo) => number> = {
  name: (a, b) => a.name.localeCompare(b.name),
  status: (a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name),
  traffic: (a, b) => b.rx_bytes + b.tx_bytes - (a.rx_bytes + a.tx_bytes) || a.name.localeCompare(b.name),
  seen: (a, b) =>
    (a.online ? -1 : (a.last_seen_secs ?? Infinity)) - (b.online ? -1 : (b.last_seen_secs ?? Infinity)) || a.name.localeCompare(b.name),
};

const COLS =
  'md:grid-cols-[minmax(0,1.6fr)_minmax(120px,1fr)_minmax(104px,1fr)_72px_80px] lg:grid-cols-[minmax(0,1.6fr)_minmax(120px,1fr)_minmax(104px,1fr)_72px_88px_80px]';
const HEAD: { label: string; className?: string }[] = [
  { label: 'Name' },
  { label: 'Address' },
  { label: 'Path' },
  { label: 'Latency', className: 'pr-3 text-right' },
  { label: 'Traffic', className: 'hidden lg:block' },
  { label: 'Last seen' },
];

export function Devices() {
  const { boot, network, peers } = useNetwork();
  const { error } = useLive();
  const { go } = useRoute();
  const [filter, setFilter] = useState<Filter>('all');
  const [sort, setSort] = useState<Sort>('status');
  const [q, setQ] = useState('');
  const net = network?.id ?? '';

  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return peers
      .filter((p) => {
        if (needle && !`${p.name} ${p.display_name} ${p.address}`.toLowerCase().includes(needle)) return false;
        if (filter === 'online') return p.online;
        if (filter === 'offline') return !p.online;
        if (filter === 'relayed') return isRelayed(p);
        if (filter === 'phones') return isPhone(p);
        return true;
      })
      .sort(ORDER[sort]);
  }, [peers, q, filter, sort]);

  const online = peers.filter((p) => p.online).length;
  const open = (p: PeerInfo) => go(`/devices/${encodeURIComponent(p.name)}`);
  const clear = () => {
    setQ('');
    setFilter('all');
  };

  return (
    <Page
      title="Devices"
      description={boot ? (peers.length === 0 ? 'No devices yet' : `${online} of ${peers.length} online`) : 'Every device on the network and how it is reached'}
      actions={<DevicesActions view="list" />}
    >
      <div className="flex flex-col gap-4">
        {boot && peers.length > 0 && (
        <div className="flex flex-wrap items-center gap-3">
          <div className="order-1 w-full min-w-0 sm:w-auto sm:flex-1">
            <Input
              value={q}
              onValueChange={(v: string) => setQ(v)}
              placeholder="Search name or address"
              aria-label="Search devices"
              className="h-8 w-full md:max-w-64"
            />
          </div>
          <div className="order-3 w-full md:order-2 md:ml-auto md:w-auto">
            <SegmentedControl value={filter} onValueChange={(v) => v && setFilter(v as Filter)} aria-label="Filter" className="flex w-full sm:w-auto">
              <SegmentedControlIndicator />
              {FILTERS.map((f) => (
                <SegmentedControlItem key={f.value} value={f.value} className="flex-1 justify-center px-0 sm:flex-none sm:px-3">{f.label}</SegmentedControlItem>
              ))}
            </SegmentedControl>
          </div>
          <div className="order-2 w-full min-w-0 sm:w-40 sm:shrink-0 md:order-3">
            <Select value={sort} onValueChange={(v) => v && setSort(v as Sort)} items={SORTS}>
              <SelectTrigger aria-label="Sort by" className="h-8 w-full min-w-0 max-w-full">
                <span className="flex min-w-0 items-center gap-1.5">
                  <span className="text-fg-3">Sort</span>
                  <SelectValue />
                </span>
              </SelectTrigger>
              <SelectContent>
                {SORTS.map((s) => (
                  <SelectItem key={s.value} value={s.value}>{s.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
        )}

        <div className="border border-line">
          {!boot && !error ? (
            <div className="divide-y divide-line" aria-hidden="true">
              {Array.from({ length: 6 }, (_, i) => (
                <div key={i} className="flex items-center gap-4 px-4 py-4">
                  <Skeleton height={14} width={`${[38, 26, 32, 22, 30, 24][i]}%`} />
                  <Skeleton height={14} width={96} className="ml-auto" />
                </div>
              ))}
            </div>
          ) : !boot ? (
            <Empty icon={<CloudOffIcon />}>Could not load devices. Check your connection and try again.</Empty>
          ) : peers.length === 0 ? (
            <Empty
              icon={<MonitorSmartphoneIcon />}
              action={<Button variant="primary" size="sm" icon={<PlusIcon />} onClick={() => go('/settings/network')}>Add device</Button>}
            >
              No devices yet. Add your first one to start the network.
            </Empty>
          ) : shown.length === 0 ? (
            <Empty action={<Button variant="ghost" size="sm" onClick={clear}>Clear filters</Button>}>No devices match these filters.</Empty>
          ) : (
            <div role="table" aria-label="Devices">
              <div role="row" className={`hidden gap-4 border-b border-line px-4 py-2 font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3 md:grid ${COLS}`}>
                {HEAD.map((h) => (
                  <span key={h.label} role="columnheader" className={`whitespace-nowrap ${h.className ?? ''}`}>{h.label}</span>
                ))}
              </div>
              <div className="divide-y divide-line">
                {shown.map((p) => {
                  const link = p.online ? linkFor(boot, net, p.node) : null;
                  const rates = p.online ? liveRates(boot, net, p.node).map((r) => r.rx + r.tx) : [];
                  const s = seen(p);
                  return (
                    <div
                      key={p.node}
                      role="row"
                      tabIndex={0}
                      onClick={() => open(p)}
                      onKeyDown={(e) => {
                        if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
                          e.preventDefault();
                          open(p);
                        }
                      }}
                      className={`grid cursor-pointer grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 px-4 py-3 outline-none transition-colors duration-150 hover:bg-ink-2 focus-visible:bg-ink-2 focus-visible:shadow-[inset_0_0_0_1px_var(--color-line-2)] ${COLS}`}
                    >
                      <div role="cell" className="flex min-w-0 items-start gap-2.5">
                        <span className="flex h-5 shrink-0 items-center"><StatusDot status={p.online ? 'online' : 'offline'} /></span>
                        <div className="min-w-0">
                          <div className="truncate text-[14px] leading-5 text-fg" title={p.name}>{p.name}</div>
                          {!sameName(p) && <div className="truncate text-[13px] leading-5 text-fg-3" title={p.display_name}>{p.display_name}</div>}
                        </div>
                      </div>
                      <div role="cell" className="min-w-0 truncate text-right font-mono text-[13px] text-fg-2 md:text-left" title={p.address}>{p.address}</div>
                      <div role="cell" className="min-w-0 justify-self-start">
                        <span className="block max-w-full truncate" title={pathText(p)}>
                          <Badge tone={p.online && !p.gateway && !isRelayed(p) ? 'strong' : 'neutral'}>{pathText(p)}</Badge>
                        </span>
                      </div>
                      <div role="cell" className="text-right font-mono text-[13px] tabular-nums md:pr-3">
                        {link?.rtt_ms != null ? (
                          <span className="text-fg-2">{fmt.ms(link.rtt_ms)}</span>
                        ) : p.online ? (
                          <span className="text-fg-3">—</span>
                        ) : (
                          <>
                            <span className="text-fg-3 md:hidden" title={s.title}>{p.last_seen_secs == null ? 'Never seen' : `Seen ${s.text}`}</span>
                            <span className="hidden text-fg-3 md:inline">—</span>
                          </>
                        )}
                      </div>
                      <div role="cell" className="hidden lg:block" title={rates.length > 1 ? 'Traffic over the last 15 minutes' : p.online ? 'No traffic in the last 15 minutes' : 'Offline'}>
                        {rates.length > 1 ? (
                          <Spark values={rates} width={88} height={20} label={`Traffic for ${p.name}, last 15 minutes`} />
                        ) : (
                          <span className="font-mono text-[13px] text-fg-3">—</span>
                        )}
                      </div>
                      <div role="cell" className="hidden truncate font-mono text-[13px] text-fg-3 md:block" title={s.title}>{s.text}</div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      </div>
    </Page>
  );
}
