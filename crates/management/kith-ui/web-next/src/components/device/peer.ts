import type { Boot, LinkTick, PeerInfo } from '@kith/types';
import { fmt } from '../../ui';

/** The newest link report about a peer: ours first, then any device's. */
export function linkFor(boot: Boot | null, network: string, node: string): LinkTick | null {
  const links = boot?.last_tick?.links ?? [];
  const me = boot?.status.node;
  return (
    links.find((l) => l.network === network && l.from === me && l.to === node) ??
    links.find((l) => l.network === network && (l.to === node || l.from === node)) ??
    null
  );
}

/** How far back "recent" traffic goes. The server's tail keeps a device's last samples however old they are. */
export const RECENT_MS = 15 * 60_000;
const BUCKET_MS = 10_000;

/**
 * Bytes per second (rx + tx) over the last 15 minutes of the snapshot's traffic tail, averaged
 * into 10 s buckets: counters move in bursts, so raw 2 s samples alternate between zero and a spike.
 * Samples older than the window (a device that went offline days ago) are dropped; counter resets skip.
 */
export function liveRates(boot: Boot | null, network: string, node: string): { t: number; rx: number; tx: number }[] {
  if (!boot) return [];
  const since = boot.server_time_ms - RECENT_MS;
  const s = (boot.traffic_tail.find((x) => x.network === network && x.node === node)?.samples ?? []).filter((x) => x.at_ms >= since);
  const buckets = new Map<number, { rx: number; tx: number; secs: number }>();
  for (let i = 1; i < s.length; i++) {
    const dt = (s[i].at_ms - s[i - 1].at_ms) / 1000;
    const rx = s[i].rx_bytes - s[i - 1].rx_bytes;
    const tx = s[i].tx_bytes - s[i - 1].tx_bytes;
    if (dt <= 0 || rx < 0 || tx < 0) continue;
    const key = Math.floor(s[i].at_ms / BUCKET_MS) * BUCKET_MS;
    const b = buckets.get(key) ?? { rx: 0, tx: 0, secs: 0 };
    buckets.set(key, { rx: b.rx + rx, tx: b.tx + tx, secs: b.secs + dt });
  }
  return [...buckets].sort((a, b) => a[0] - b[0]).map(([t, b]) => ({ t, rx: b.rx / b.secs, tx: b.tx / b.secs }));
}

/** The display name says nothing the name doesn't, such as "Desk" for desk or "Lays-MacBook-Pro" for lays-macbook-pro. */
export const sameName = (p: PeerInfo) => {
  const norm = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]/gu, '');
  return !p.display_name || norm(p.display_name) === norm(p.name);
};

export const isPhone = (p: PeerInfo) => p.gateway != null || /phone|iphone|android|pixel/i.test(p.name);

/** Online but not on a direct path. */
export const isRelayed = (p: PeerInfo) => p.online && (p.path === 'relay' || p.path === 'stealth');

export function pathText(p: PeerInfo, relay?: string | null): string {
  if (!p.online) return 'Offline';
  if (p.gateway) return `Via ${p.gateway}`;
  if (p.path === 'relay') return relay ? `Relayed via ${relay}` : 'Relayed';
  if (p.path === 'stealth') return 'Stealth';
  return 'Direct';
}

export function seen(p: PeerInfo): { text: string; title?: string } {
  if (p.online) return { text: 'Now' };
  if (p.last_seen_secs == null) return { text: 'Never' };
  const t = Date.now() - p.last_seen_secs * 1000;
  return { text: fmt.relative(t), title: fmt.absolute(t) };
}
