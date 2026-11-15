import type { PeerInfo } from '@kith/types';

// Policy shapes, as GET /api/networks/:net/policy returns them (web/src/lib/types.ts).
// The API sends each selector as {kind, name, value}: a tag's name, or a device's name and node id.
export type Selector = { kind: 'any' | 'device' | 'tag'; name: string | null; value: string | null };
type Target = { selector: Selector; protocol: 'any' | 'tcp' | 'udp' | 'icmp'; ports: { start: number; end: number }[] | null };
export type Rule = { sources: Selector[]; destinations: Target[] };
export type PolicyView = { toml: string; rules: Rule[] };

export const selKey = (s: Selector) => (s.kind === 'any' ? '*' : s.kind === 'tag' ? `tag:${s.name ?? s.value}` : `dev:${s.value}`);

export function selName(s: Selector, peers: PeerInfo[]) {
  if (s.kind === 'any') return 'everyone';
  if (s.kind === 'tag') return s.name ?? s.value ?? 'tag';
  return s.name ?? peers.find((p) => p.node === s.value)?.name ?? (s.value ? `device ${s.value.slice(0, 8)}` : 'a removed device');
}

export function portText(t: Target) {
  if (t.protocol === 'icmp') return 'ICMP ping';
  const ports = t.ports?.map((r) => (r.start === r.end ? `${r.start}` : `${r.start}-${r.end}`)).join(', ');
  if (!ports) return t.protocol === 'any' ? 'any port' : `any ${t.protocol.toUpperCase()} port`;
  if (t.protocol === 'any') return `${t.ports && t.ports.length > 1 ? 'ports' : 'port'} ${ports}`;
  return `${t.protocol.toUpperCase()} ${ports}`;
}

/** Whether a source selector covers row `key`, and a destination covers column `key`. */
export const covers = (s: Selector, key: string) => s.kind === 'any' || selKey(s) === key;

/** Labels for the matrix axes: every selector the policy names, `*` first. */
export function axes(rules: Rule[]) {
  const keys = new Map<string, Selector>();
  for (const r of rules) {
    for (const s of r.sources) keys.set(selKey(s), s);
    for (const d of r.destinations) keys.set(selKey(d.selector), d.selector);
  }
  return [...keys.values()].sort((a, b) => selKey(a).localeCompare(selKey(b)));
}
