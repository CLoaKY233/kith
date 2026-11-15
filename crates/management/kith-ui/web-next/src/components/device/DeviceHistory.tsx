import { useMemo } from 'react';
import type { JournalPage, StoredEntry } from '@kith/types';
import { Timeline, TimelineDescription, TimelineItem, TimelineMarker, TimelineTime, TimelineTitle } from '@ui-sdk/react/timeline';
import { useFetch, useNetwork } from '../../lib/data';
import { Empty, Panel, Skeleton, fmt } from '../../ui';

const PATHS: Record<string, string> = { direct: 'directly', relay: 'through a relay', stealth: 'in stealth mode' };
const via = (p: string | null | undefined) => (p ? (PATHS[p] ?? p) : 'not connected');

function sentence(e: StoredEntry['entry']): { text: string; bad?: boolean } {
  switch (e.kind) {
    case 'peer_online': return { text: `Came online${e.path ? ` ${via(e.path)}` : ''}` };
    case 'peer_offline': return { text: 'Went offline' };
    case 'path_changed': return { text: e.to ? `Now reached ${via(e.to)}, was ${via(e.from)}` : `Lost its connection${e.from ? `, was reached ${via(e.from)}` : ''}` };
    case 'endpoint_changed': return { text: 'Changed network' };
    case 'device_joined': return { text: `Joined as ${e.address}` };
    case 'device_removed': return { text: 'Removed from the network', bad: true };
    case 'device_renamed': return { text: `Renamed from ${e.old_name} to ${e.name}` };
    case 'join_requested': return { text: 'Asked to join' };
    case 'blocked': return { text: `Blocked ${e.protocol.toUpperCase()}${e.port ? ` port ${e.port}` : ''} from ${e.from_name} to ${e.to_name}, ${e.count === 1 ? 'once' : `${e.count} times`}`, bad: true };
    default: {
      const k = e.kind.replace(/_/g, ' ');
      return { text: k[0].toUpperCase() + k.slice(1) };
    }
  }
}

function involves(e: StoredEntry['entry'], node: string) {
  const x = e as unknown as Record<string, unknown>;
  return x.node === node || x.from === node || x.to === node;
}

export function DeviceHistory({ node, span }: { node: string; span?: 12 }) {
  const { network } = useNetwork();
  const { data, error } = useFetch<JournalPage>(network ? `/api/history/journal?${new URLSearchParams({ network: network.id, limit: '1000' })}` : null, 15_000);
  const entries = data?.entries;
  const rows = useMemo(() => (entries ?? []).filter((s) => involves(s.entry, node)).slice(0, 30), [entries, node]);
  return (
    <Panel label="History" span={span}>
      {!data && !error ? (
        <div className="flex flex-col gap-3">
          {Array.from({ length: 4 }, (_, i) => <Skeleton key={i} height={20} />)}
        </div>
      ) : !data ? (
        <Empty>History is not available right now.</Empty>
      ) : rows.length === 0 ? (
        <Empty>Nothing recorded for this device yet.</Empty>
      ) : (
        <Timeline aria-label="Device history" className="flex flex-col">
          {rows.map((s) => {
            const { text, bad } = sentence(s.entry);
            return (
              <TimelineItem key={s.id} className="grid grid-cols-[12px_1fr_auto] items-baseline gap-x-3 border-l border-line py-2 pl-0">
                <TimelineMarker className={`-ml-[4px] size-[7px] self-center ${bad ? 'bg-hot' : 'bg-fg-3'}`} />
                <TimelineTitle className={`min-w-0 text-[14px] [overflow-wrap:anywhere] ${bad ? 'text-hot' : 'text-fg'}`}>{text}</TimelineTitle>
                <TimelineTime dateTime={new Date(s.entry.at_ms).toISOString()} title={fmt.absolute(s.entry.at_ms)} className="whitespace-nowrap font-mono text-[12px] tabular-nums text-fg-3">
                  {fmt.relative(s.entry.at_ms)}
                </TimelineTime>
                <TimelineDescription className="sr-only">{fmt.absolute(s.entry.at_ms)}</TimelineDescription>
              </TimelineItem>
            );
          })}
        </Timeline>
      )}
    </Panel>
  );
}
