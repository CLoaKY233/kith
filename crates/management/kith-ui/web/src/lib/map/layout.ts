// The map's ring layout (spec §9.2): a pure function of the node-id set, so the same
// devices always land in the same places. Workstream F owns it.

export type MapNode = { id: string; x: number; y: number; ring: 'self' | 'outer' | 'inner' };

function hash(id: string): number {
  let h = 2166136261;
  for (let i = 0; i < id.length; i += 1) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** Places `self` at the centre and `peers` evenly on a ring, ordered by hash rank. */
export function layout(self: string, peers: readonly string[], radius: number): MapNode[] {
  const ranked = [...peers].sort((a, b) => hash(a) - hash(b) || a.localeCompare(b));
  const nodes: MapNode[] = [{ id: self, x: 0, y: 0, ring: 'self' }];
  ranked.forEach((id, index) => {
    const angle = (index / Math.max(1, ranked.length)) * 2 * Math.PI - Math.PI / 2;
    nodes.push({ id, x: Math.cos(angle) * radius, y: Math.sin(angle) * radius, ring: 'outer' });
  });
  return nodes;
}
