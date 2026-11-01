import { describe, expect, it } from 'vitest';
import {
  between,
  curve,
  DENSE_OVER,
  fromTop,
  layout,
  START_ANGLE,
  truncate,
  turn,
  type Geometry,
  type LayoutInput,
  type Placed,
} from './layout';

const GEOMETRY: Geometry = { radius: 200, satellite: 28, spacing: 20 };
const TAU = 2 * Math.PI;
const RUNS = 300;

/** mulberry32: a small seeded generator, so a failing case can be replayed. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 2 ** 32;
  };
}

function id(next: () => number): string {
  return Array.from({ length: 16 }, () => Math.floor(next() * 16).toString(16)).join('');
}

function ids(next: () => number, n: number): string[] {
  return [...new Set(Array.from({ length: n }, () => id(next)))];
}

function shuffle<T>(items: readonly T[], next: () => number): T[] {
  const out = [...items];
  for (let i = out.length - 1; i > 0; i -= 1) {
    const j = Math.floor(next() * (i + 1));
    [out[i], out[j]] = [out[j] as T, out[i] as T];
  }
  return out;
}

function at(placed: ReadonlyMap<string, Placed>, key: string): Placed {
  const node = placed.get(key);
  if (!node) throw new Error(`missing ${key}`);
  return node;
}

/** Ids on the given ring in clockwise order from the top. */
function clockwise(placed: ReadonlyMap<string, Placed>, ring: Placed['ring']): string[] {
  return [...placed.values()]
    .filter((n) => n.ring === ring)
    .sort((a, b) => fromTop(a.angle) - fromTop(b.angle))
    .map((n) => n.id);
}

/** True when `small` appears in `big` in the same cyclic order. */
function sameCycle(small: readonly string[], big: readonly string[]): boolean {
  const kept = big.filter((x) => small.includes(x));
  if (kept.length !== small.length) return false;
  if (kept.length === 0) return true;
  const start = kept.indexOf(small[0] as string);
  return small.every((x, i) => kept[(start + i) % kept.length] === x);
}

describe('map layout (spec §9.2)', () => {
  it('is a function of the id set, not of input order', () => {
    const next = rng(1);
    for (let run = 0; run < RUNS; run += 1) {
      const self = id(next);
      const peers = ids(next, 1 + Math.floor(next() * 40));
      const a = layout({ self, peers }, GEOMETRY);
      const b = layout({ self, peers: shuffle(peers, next) }, GEOMETRY);
      expect([...b.nodes.entries()].sort()).toEqual([...a.nodes.entries()].sort());
      expect(b.order).toEqual(a.order);
    }
  });

  it('puts this device at the centre and spaces peers evenly on the ring', () => {
    const next = rng(2);
    for (let run = 0; run < RUNS; run += 1) {
      const self = id(next);
      const peers = ids(next, 1 + Math.floor(next() * DENSE_OVER));
      const { nodes } = layout({ self, peers }, GEOMETRY);
      expect(at(nodes, self)).toMatchObject({ ring: 'self', x: 0, y: 0 });
      const ring = clockwise(nodes, 'outer');
      expect(ring).toHaveLength(peers.length);
      expect(fromTop(at(nodes, ring[0] as string).angle)).toBeCloseTo(0);
      ring.forEach((key, i) => {
        const node = at(nodes, key);
        expect(Math.hypot(node.x, node.y)).toBeCloseTo(GEOMETRY.radius);
        expect(fromTop(node.angle)).toBeCloseTo((TAU * i) / ring.length);
      });
    }
  });

  it('moves nobody more than one slot or out of order when a device is added or removed', () => {
    const next = rng(3);
    for (let run = 0; run < RUNS; run += 1) {
      const self = id(next);
      const before = ids(next, 1 + Math.floor(next() * (DENSE_OVER - 1)));
      const added = id(next);
      const after = [...before, added];
      const a = layout({ self, peers: before }, GEOMETRY);
      const b = layout({ self, peers: after }, GEOMETRY);
      const slot = TAU / after.length;
      for (const key of before) {
        expect(Math.abs(turn(at(a.nodes, key).angle, at(b.nodes, key).angle))).toBeLessThanOrEqual(slot + 1e-9);
      }
      expect(sameCycle(clockwise(a.nodes, 'outer'), clockwise(b.nodes, 'outer'))).toBe(true);
      // Removing it again restores the first picture exactly.
      expect(layout({ self, peers: before }, GEOMETRY).nodes).toEqual(a.nodes);
    }
  });

  it('slides along the ring, never through the middle', () => {
    const next = rng(4);
    for (let run = 0; run < RUNS; run += 1) {
      const self = id(next);
      const before = ids(next, 2 + Math.floor(next() * 20));
      const a = layout({ self, peers: before }, GEOMETRY);
      const b = layout({ self, peers: [...before, id(next)] }, GEOMETRY);
      for (const key of before) {
        for (const t of [0, 0.25, 0.5, 0.75, 1]) {
          const p = between(at(a.nodes, key), at(b.nodes, key), t);
          expect(Math.hypot(p.x, p.y)).toBeCloseTo(GEOMETRY.radius);
        }
        const end = between(at(a.nodes, key), at(b.nodes, key), 1);
        expect(end.x).toBeCloseTo(at(b.nodes, key).x);
        expect(end.y).toBeCloseTo(at(b.nodes, key).y);
      }
    }
  });

  it('gives every device its own place', () => {
    const next = rng(5);
    for (let run = 0; run < RUNS; run += 1) {
      const self = id(next);
      const peers = ids(next, 1 + Math.floor(next() * 70));
      const gateways = new Map<string, string>();
      for (const p of peers) if (next() < 0.2) gateways.set(p, peers[Math.floor(next() * peers.length)] as string);
      const relays = new Map<string, string[]>();
      for (const p of peers) if (next() < 0.1) relays.set(p, peers.filter(() => next() < 0.3));
      const online = new Set(peers.filter(() => next() < 0.7));
      const { nodes, order } = layout({ self, peers, gateways, relays, online }, GEOMETRY);
      expect(nodes.size).toBe(peers.length + 1);
      expect(new Set(order).size).toBe(order.length);
      expect(order[0]).toBe(self);
      const spots = new Set([...nodes.values()].map((n) => `${n.x.toFixed(3)},${n.y.toFixed(3)}`));
      expect(spots.size).toBe(nodes.size);
    }
  });

  it('hangs phones 28 px outward from their gateway', () => {
    const next = rng(6);
    for (let run = 0; run < RUNS; run += 1) {
      const self = id(next);
      const peers = ids(next, 3 + Math.floor(next() * 15));
      const [gateway, ...rest] = peers;
      const phones = rest.filter(() => next() < 0.4);
      const gateways = new Map(phones.map((p) => [p, gateway as string]));
      const { nodes } = layout({ self, peers, gateways }, GEOMETRY);
      const g = at(nodes, gateway as string);
      for (const phone of phones) {
        const p = at(nodes, phone);
        expect(p).toMatchObject({ ring: 'satellite', gateway });
        expect(Math.hypot(p.x - g.x, p.y - g.y)).toBeCloseTo(GEOMETRY.satellite);
        expect(Math.hypot(p.x, p.y)).toBeGreaterThan(GEOMETRY.radius);
      }
    }
  });

  it('keeps phones on the ring when their gateway is this device, missing or itself a phone', () => {
    const self = 'self';
    const input: LayoutInput = {
      self,
      peers: ['a', 'b', 'c', 'd'],
      gateways: new Map([
        ['a', self],
        ['b', 'gone'],
        ['c', 'd'],
        ['d', 'c'],
      ]),
    };
    const { nodes } = layout(input, GEOMETRY);
    for (const key of ['a', 'b', 'c', 'd']) expect(at(nodes, key).ring).toBe('outer');
  });

  it('puts a relay on the inner ring at the mean angle of its users', () => {
    const next = rng(7);
    for (let run = 0; run < RUNS; run += 1) {
      const self = id(next);
      const peers = ids(next, 4 + Math.floor(next() * 16));
      const [relay, ...rest] = peers;
      const users = rest.slice(0, 1 + Math.floor(next() * 2));
      const { nodes } = layout({ self, peers, relays: new Map([[relay as string, users]]) }, GEOMETRY);
      const r = at(nodes, relay as string);
      expect(r.ring).toBe('relay');
      expect(Math.hypot(r.x, r.y)).toBeLessThan(GEOMETRY.radius);
      const sx = users.reduce((s, u) => s + Math.cos(at(nodes, u).angle), 0);
      const sy = users.reduce((s, u) => s + Math.sin(at(nodes, u).angle), 0);
      if (Math.hypot(sx, sy) > 1e-6) expect(Math.abs(turn(r.angle, Math.atan2(sy, sx)))).toBeLessThan(1e-9);
    }
  });

  it('keeps two relays with the same users apart', () => {
    const relays = new Map([
      ['r1', ['a']],
      ['r2', ['a']],
    ]);
    const { nodes } = layout({ self: 's', peers: ['a', 'b', 'r1', 'r2'], relays }, GEOMETRY);
    const r1 = at(nodes, 'r1');
    const r2 = at(nodes, 'r2');
    expect(Math.hypot(r1.x - r2.x, r1.y - r2.y)).toBeGreaterThanOrEqual(GEOMETRY.spacing - 1e-9);
  });

  it(`moves online devices to an inner ring above ${DENSE_OVER} devices`, () => {
    const next = rng(8);
    const peers = ids(next, DENSE_OVER + 6);
    const online = new Set(peers.slice(0, 20));
    const dense = layout({ self: 's', peers, online }, GEOMETRY);
    expect(dense.dense).toBe(true);
    for (const p of peers) {
      const n = at(dense.nodes, p);
      expect(n.ring).toBe(online.has(p) ? 'inner' : 'outer');
      if (online.has(p)) expect(Math.hypot(n.x, n.y)).toBeLessThan(GEOMETRY.radius);
    }
    const sparse = layout({ self: 's', peers: peers.slice(0, DENSE_OVER), online }, GEOMETRY);
    expect(sparse.dense).toBe(false);
    expect(clockwise(sparse.nodes, 'inner')).toHaveLength(0);
  });

  it('orders keys clockwise from the top', () => {
    const { order, nodes } = layout({ self: 's', peers: ['a', 'b', 'c', 'd', 'e'] }, GEOMETRY);
    const angles = order.slice(1).map((k) => fromTop(at(nodes, k).angle));
    expect(angles).toEqual([...angles].sort((a, b) => a - b));
    expect(START_ANGLE).toBe(-Math.PI / 2);
  });

  it('follows the CSS easing curve', () => {
    const pop = curve('cubic-bezier(0.2, 0.8, 0.2, 1)');
    expect(pop(0)).toBe(0);
    expect(pop(1)).toBe(1);
    expect(pop(0.5)).toBeGreaterThan(0.5);
    for (let t = 0; t < 1; t += 0.05) expect(pop(t + 0.05)).toBeGreaterThanOrEqual(pop(t));
    expect(curve('linear')(0.3)).toBeCloseTo(0.3);
  });

  it('cuts labels at 14 characters', () => {
    expect(truncate('work-laptop')).toBe('work-laptop');
    expect(truncate('abcdefghijklmn')).toBe('abcdefghijklmn');
    expect(truncate('abcdefghijklmnop')).toBe('abcdefghijklm…');
    expect([...truncate('abcdefghijklmnop')]).toHaveLength(14);
  });
});
