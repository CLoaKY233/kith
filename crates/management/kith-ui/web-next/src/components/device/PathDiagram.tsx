import type { LinkTick, PeerInfo } from '@kith/types';
import { fmt } from '../../ui';

/** This device → [relay] → [gateway] → the peer, with the measured round trip. */
export function PathDiagram({ me, peer, link, relay, self = false }: { me: string; peer: PeerInfo; link: LinkTick | null; relay: string | null; self?: boolean }) {
  const relayed = peer.online && (peer.path === 'relay' || peer.path === 'stealth');
  const hops: { name: string; role: string }[] = self
    ? [{ name: peer.name, role: 'This device' }]
    : [
        { name: me, role: 'This device' },
        ...(relayed ? [{ name: relay ?? 'Unknown', role: 'Relay' }] : []),
        ...(peer.gateway ? [{ name: peer.gateway, role: 'Gateway' }] : []),
        { name: peer.name, role: peer.gateway ? 'Phone' : 'Device' },
      ];
  const color = self ? 'var(--color-fg)' : !peer.online ? 'var(--color-fg-3)' : relayed || peer.gateway ? 'var(--color-fg-2)' : 'var(--color-fg)';
  const summary = self
    ? 'This is the device you are using'
    : !peer.online
    ? 'No path while offline'
    : [peer.path === 'relay' ? 'Relayed' : peer.path === 'stealth' ? 'Stealth' : 'Direct', link?.rtt_ms != null && fmt.ms(link.rtt_ms), link?.loss ? `${fmt.percent(link.loss * 100)} loss` : null]
        .filter(Boolean)
        .join(' · ');
  return (
    <figure className="flex flex-col gap-4">
      <figcaption className="font-mono text-[13px]" style={{ color }}>{summary}</figcaption>
      <ol className="flex w-full items-start gap-1 sm:gap-2" aria-label="Path">
        {hops.map((h, i) => {
          const end = i === 0 || i === hops.length - 1;
          const live = self || peer.online;
          return [
            i > 0 && (
              <li key={`l${i}`} aria-hidden="true" className="mt-[12px] min-w-3 flex-1 border-t" style={{ borderColor: color, borderStyle: relayed || !peer.online ? 'dashed' : 'solid' }} />
            ),
            <li key={`n${i}`} title={`${h.role}: ${h.name}`} className="flex w-20 min-w-0 shrink flex-col items-center gap-1 text-center sm:w-28">
              <span className="grid size-6 place-items-center border bg-ink-0" style={{ borderColor: end ? color : 'var(--color-line-2)' }}>
                {end && live && <span className="size-2" style={{ background: color }} />}
              </span>
              <span className="mt-1 max-w-full truncate font-mono text-[13px] text-fg" title={h.name}>{h.name}</span>
              <span className="font-mono text-[10px] uppercase tracking-[0.04em] text-fg-3 [overflow-wrap:anywhere] sm:text-[11px] sm:tracking-[0.08em]">{h.role}</span>
            </li>,
          ];
        })}
      </ol>
      {peer.path_reason && <p className="text-[13px] text-fg-3 [overflow-wrap:anywhere]">{peer.path_reason}</p>}
    </figure>
  );
}
