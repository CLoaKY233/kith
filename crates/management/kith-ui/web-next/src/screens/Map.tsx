import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { RelayView } from '@kith/types';
import { Toggle } from '@ui-sdk/react/toggle';
import { SegmentedControl, SegmentedControlIndicator, SegmentedControlItem } from '@ui-sdk/react/segmented-control';
import { ArrowUpRightIcon, LocateFixedIcon, MinusIcon, NetworkIcon, PlusIcon, XIcon } from 'lucide-react';
import { useFetch, useLive, useNetwork } from '../lib/data';
import { net } from '../lib/api';
import { useRoute } from '../lib/router';
import { Badge, Button, Empty, Page, Panel, Skeleton, Spark, StatusDot, fmt } from '../ui';
import { MapCanvas, edgeLive, edgeVisible, layout, nodeShown, type MapControls, type MapEdge, type MapFilter, type MapNode } from '../components/MapGraph';
import { DevicesActions } from '../components/device/DevicesActions';
import { liveRates, pathText, seen } from '../components/device/peer';

const FILTERS: { value: MapFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'direct', label: 'Direct' },
  { value: 'relayed', label: 'Relayed' },
  { value: 'offline', label: 'Offline' },
];

const FILTER_EMPTY: Record<MapFilter, string> = {
  all: '',
  direct: 'No direct links right now.',
  relayed: 'No relayed links right now.',
  offline: 'No devices are offline.',
};

/** Height that fills the viewport below the element's top edge. */
function useFill<T extends HTMLElement>(bottom = 32, min = 420) {
  const ref = useRef<T>(null);
  const [h, setH] = useState(min);
  useLayoutEffect(() => {
    const set = () => {
      const el = ref.current;
      if (el) {
        const fill = window.innerHeight - el.getBoundingClientRect().top - bottom;
        setH(Math.round(Math.max(min, window.innerWidth < 768 ? Math.min(fill, el.clientWidth * 1.0) : fill)));
      }
    };
    set();
    addEventListener('resize', set);
    const ro = new ResizeObserver(set);
    ro.observe(document.documentElement);
    if (ref.current?.parentElement) ro.observe(ref.current.parentElement);
    return () => {
      removeEventListener('resize', set);
      ro.disconnect();
    };
  }, [bottom, min]);
  return [ref, h] as const;
}

export function MapScreen() {
  const { boot, network, peers } = useNetwork();
  const { error } = useLive();
  const relaysQ = useFetch<RelayView[]>(network ? net(network.id, '/relays') : null, 15000);
  const [filter, setFilter] = useState<MapFilter>('all');
  const [allLinks, setAllLinks] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const controls = useRef<MapControls>(null);
  
  const meId = boot?.status.node ?? 'me';
  const meName = boot?.status.host.name ?? 'This device';
  const exitName = boot?.status.exit_node?.split('/').pop() ?? null;
  const links = useMemo(() => (boot?.last_tick?.links ?? []).filter((l) => !network || l.network === network.id), [boot, network]);
  const relaySet = useMemo(() => new Set((relaysQ.data ?? []).map((r) => r.node)), [relaysQ.data]);
  const { nodes, edges: all } = useMemo(
    () => layout({ id: meId, name: meName }, peers, links, relaySet, exitName),
    [meId, meName, peers, links, relaySet, exitName],
  );
  const meshCount = all.filter((e) => e.kind === 'mesh').length;
  const edges = useMemo(() => (allLinks ? all : all.filter((e) => e.kind !== 'mesh')), [all, allLinks]);
  const onScreen = edges.filter((e) => edgeVisible(e, filter));

  const counts: Record<MapFilter, number> = {
    all: peers.length,
    direct: edges.filter((e) => edgeVisible(e, 'direct') && e.kind !== 'internet').length,
    relayed: edges.filter((e) => e.kind === 'relayed').length,
    offline: edges.filter((e) => e.kind === 'offline').length,
  };
  const node = nodes.find((n) => n.id === selected && n.kind !== 'internet' && nodeShown(n, edges, filter)) ?? null;
  const relayName = useMemo(() => {
    const via = node && links.find((l) => l.from === meId && l.to === node.id)?.relay;
    return via ? (nodes.find((x) => x.id === via)?.name ?? 'a relay') : null;
  }, [node, links, meId, nodes]);
  const hasMap = !!boot && peers.length > 0;
  // The legend sits under the canvas, so leave room for it below.
  const [fillRef, height] = useFill<HTMLDivElement>(hasMap ? 84 : 32);
  const online = peers.filter((p) => p.online).length;
  useEffect(() => {
    if (boot && selected && !node) setSelected(null);
  }, [boot, selected, node]);
  useEffect(() => {
    if (!selected) return;
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setSelected(null);
    addEventListener('keydown', esc);
    return () => removeEventListener('keydown', esc);
  }, [selected]);

  return (
    <Page
      title="Devices"
      description={hasMap ? `${online} of ${peers.length} online${exitText(exitName, peers)}` : 'How every device reaches this one'}
      actions={<DevicesActions view="map" />}
    >
      <div className="flex flex-col gap-3">
        {(!boot || hasMap) && (
        <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
          <div className="max-w-full overflow-x-auto [&>*]:items-center">
            <SegmentedControl aria-label="Show links" className="h-8" value={filter} onValueChange={(v) => v && setFilter(v as MapFilter)}>
              <SegmentedControlIndicator />
              {FILTERS.map((f) => (
                <SegmentedControlItem key={f.value} value={f.value} className="h-[30px] gap-1.5 px-2 sm:px-3">
                  {f.label}<span className="font-mono text-fg-3">{counts[f.value]}</span>
                </SegmentedControlItem>
              ))}
            </SegmentedControl>
          </div>
          <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Map view">
            {meshCount > 0 && (
              <Toggle pressed={allLinks} onPressedChange={setAllLinks} className="h-8 gap-2">
                All links <span className="font-mono text-fg-3">{meshCount}</span>
              </Toggle>
            )}
            <Button icon={<MinusIcon />} aria-label="Zoom out" disabled={!hasMap} onClick={() => controls.current?.zoom(0.8)} />
            <Button icon={<PlusIcon />} aria-label="Zoom in" disabled={!hasMap} onClick={() => controls.current?.zoom(1.25)} />
            <Button icon={<LocateFixedIcon />} disabled={!hasMap} onClick={() => controls.current?.fit()}>
              Fit
            </Button>
          </div>
        </div>
        )}

        <div ref={fillRef} className="relative overflow-hidden border border-line" style={{ height }}>
          {!boot ? (
            error ? (
              <div className="grid h-full place-items-center">
                <Empty icon={<NetworkIcon />}>
                  <span title={error}>Can't reach the console right now. Trying again.</span>
                </Empty>
              </div>
            ) : (
              <Skeleton height={height} />
            )
          ) : peers.length === 0 ? (
            <div className="grid h-full place-items-center">
              <Empty icon={<NetworkIcon />}>No other devices yet. Add one to see how it connects.</Empty>
            </div>
          ) : (
            <MapCanvas ref={controls} nodes={nodes} edges={edges} filter={filter} selected={selected} onSelect={setSelected} />
          )}

          {hasMap && filter !== 'all' && counts[filter] === 0 && (
            <p className="pointer-events-none absolute inset-x-0 top-4 text-center text-[13px] text-fg-3">{FILTER_EMPTY[filter]}</p>
          )}

          {node && boot && network && (
            <div className="absolute inset-x-3 bottom-3 max-h-[60%] overflow-auto md:inset-x-auto md:bottom-auto md:right-3 md:top-3 md:max-h-[calc(100%-24px)] md:w-80">
              <NodePanel
                n={node}
                rates={node.peer?.online ? liveRates(boot, network.id, node.id).map((r) => r.rx + r.tx) : []}
                relay={relayName}
                onClose={() => setSelected(null)}
              />
            </div>
          )}
        </div>
        {hasMap && <Legend edges={onScreen} />}
      </div>
    </Page>
  );
}

/** Names the exit only when it is a device here, and says so when that device is offline. */
function exitText(exitName: string | null, peers: { name: string; online: boolean }[]) {
  const exit = exitName ? peers.find((p) => p.name === exitName) : undefined;
  if (!exit) return '';
  return exit.online ? `, internet through ${exit.name}` : `, exit ${exit.name} is offline`;
}

function Legend({ edges }: { edges: MapEdge[] }) {
  const has = (k: MapEdge['kind']) => edges.some((e) => e.kind === k);
  if (!edges.length) return null;
  const row = (stroke: string, text: string, dash?: string, width = 1) => (
    <div className="flex min-w-0 items-center gap-2.5">
      <svg width="24" height="8" className="shrink-0" aria-hidden="true">
        <line x1="0" y1="4" x2="24" y2="4" stroke={stroke} strokeWidth={width} strokeDasharray={dash} />
      </svg>
      <span className="whitespace-nowrap">{text}</span>
    </div>
  );
  return (
    <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-[12px] leading-4 text-fg-2" aria-label="Legend">
        {has('direct') && row('var(--color-fg)', 'Direct')}
        {has('relayed') && row('var(--color-fg-2)', 'Relayed', '6 5')}
        {has('gateway') && row('var(--color-fg-2)', 'Phone via gateway')}
        {has('offline') && row('var(--color-fg-3)', 'Offline', '1 5')}
        {has('internet') && row('var(--color-fg)', 'Exit to internet', '8 3')}
        {has('mesh') && row('var(--color-fg-3)', 'Between devices')}
        {edges.some(edgeLive) && row('var(--color-fg)', 'Traffic now', '2 10', 2)}
    </div>
  );
}

function NodePanel({ n, rates, relay, onClose }: { n: MapNode; rates: number[]; relay: string | null; onClose: () => void }) {
  const { go } = useRoute();
  const p = n.peer;
  const row = (k: string, v: ReactNode, title?: string) => (
    <div className="flex items-baseline justify-between gap-3 py-1.5">
      <dt className="font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">{k}</dt>
      <dd className="min-w-0 truncate text-right font-mono text-[13px] text-fg" title={title}>{v}</dd>
    </div>
  );
  const s = p ? seen(p) : null;
  return (
    <div className="bg-ink-0" role="region" aria-label={`${n.name} details`}>
      <Panel
        label={n.kind === 'me' ? 'This device' : n.kind === 'phone' ? 'Phone' : 'Device'}
        right={<Button variant="ghost" size="sm" className="-mr-2 w-7 px-0" icon={<XIcon />} aria-label="Close" onClick={onClose} />}
      >
        <div className="flex flex-col gap-4">
          <div>
            <div className="flex items-center gap-2">
              <StatusDot status={n.online ? 'online' : 'offline'} />
              <h2 className="min-w-0 truncate text-[20px] font-medium text-fg" title={n.name}>{n.name}</h2>
            </div>
            {(n.exit || n.relay) && (
              <div className="mt-2 flex flex-wrap gap-1.5">
                {n.exit && <Badge tone="strong">Exit</Badge>}
                {n.relay && <Badge>Relay</Badge>}
              </div>
            )}
          </div>
          {p ? (
            <>
              <dl className="divide-y divide-line">
                {row('Address', p.address, p.address)}
                {row('Path', pathText(p, relay), pathText(p, relay))}
                {row('Latency', n.rtt != null ? fmt.ms(n.rtt) : '—')}
                {row('Now', n.rate != null ? fmt.rate(n.rate) : '—')}
                {row('From it', fmt.bytes(p.rx_bytes))}
                {row('To it', fmt.bytes(p.tx_bytes))}
                {row('Last seen', s!.text, s!.title)}
              </dl>
              {p.path_reason && <p className="text-[13px] text-fg-3 [overflow-wrap:anywhere]">{p.path_reason}</p>}
              <div>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.08em] text-fg-3">Traffic, recent</div>
                {rates.length > 1 ? <Spark values={rates} width={288} height={40} label="Traffic, last 15 minutes" /> : <p className="text-[13px] text-fg-3">{n.peer?.online ? 'No traffic in the last 15 minutes.' : 'Offline, so no recent traffic.'}</p>}
              </div>
              <Button variant="primary" icon={<ArrowUpRightIcon />} onClick={() => go(`/devices/${encodeURIComponent(p.name)}`)}>
                Open device
              </Button>
            </>
          ) : (
            <p className="text-[13px] text-fg-2">The device this dashboard runs on. Every link on the map is measured from here.</p>
          )}
        </div>
      </Panel>
    </div>
  );
}
