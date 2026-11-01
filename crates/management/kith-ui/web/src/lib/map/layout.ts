// The map's ring layout (spec §9.2). A pure function of the node-id set and the few
// structural facts that place a node (which devices relay, which hang off a gateway, and,
// above DENSE_OVER devices, which are online). The same input always gives the same
// picture, so map nodes never move unless that input changes. No simulation, no stored
// positions.

/** Above this many devices, online devices take an inner ring and offline ones the outer. */
export const DENSE_OVER = 24;
/** Above this many devices the map shows as a table by default. */
export const TABLE_OVER = 60;
/** `Show all links` is offered only below this many devices. */
export const ALL_LINKS_UNDER = 15;
/** Map labels are cut to this many characters. */
export const LABEL_MAX = 14;
/** The typographic mark that ends a cut label. */
const ELLIPSIS = '\u2026';

/** Ring radii as a share of the outer radius. */
const RELAY_RING = 0.45;
const DENSE_RELAY_RING = 0.34;
const DENSE_ONLINE_RING = 0.68;

/** The first slot is at the top; slots run clockwise (SVG y points down). */
export const START_ANGLE = -Math.PI / 2;
const TAU = 2 * Math.PI;

export type Ring = 'self' | 'outer' | 'inner' | 'relay' | 'satellite';

export interface LayoutInput {
  /** This device; it sits at the centre. */
  readonly self: string;
  /** Every other device in the network. */
  readonly peers: readonly string[];
  /** Relay id → the devices whose path goes through it. */
  readonly relays?: ReadonlyMap<string, readonly string[]>;
  /** Device id → the id of the device it connects through (phones). */
  readonly gateways?: ReadonlyMap<string, string>;
  /** Devices online now. Read only above DENSE_OVER devices. */
  readonly online?: ReadonlySet<string>;
}

export interface Geometry {
  /** Outer ring radius, in px. */
  readonly radius: number;
  /** How far a satellite sits from its gateway, in px (`--map-satellite`). */
  readonly satellite: number;
  /** The smallest gap between two satellites or two relays, in px. */
  readonly spacing: number;
}

export interface Placed {
  readonly id: string;
  readonly ring: Ring;
  /** Polar position around the centre: angle in radians, distance in px. */
  readonly angle: number;
  readonly r: number;
  readonly x: number;
  readonly y: number;
  /** For a satellite: the device it hangs off. */
  readonly gateway: string | null;
}

export interface Layout {
  readonly nodes: ReadonlyMap<string, Placed>;
  /** Keyboard order: this device, then clockwise from the top, inner before outer. */
  readonly order: readonly string[];
  /** True above DENSE_OVER devices. */
  readonly dense: boolean;
}

/** FNV-1a, 32 bit: a stable rank key that does not depend on input order. */
export function hash(id: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i += 1) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

function byRank(a: string, b: string): number {
  return hash(a) - hash(b) || (a < b ? -1 : a > b ? 1 : 0);
}

/** An angle as a clockwise offset from the top, in [0, 2π). */
export function fromTop(angle: number): number {
  return (((angle - START_ANGLE) % TAU) + TAU) % TAU;
}

/** The signed shortest turn from `a` to `b`, in (-π, π]. */
export function turn(a: number, b: number): number {
  const d = (((b - a) % TAU) + TAU) % TAU;
  return d > Math.PI ? d - TAU : d;
}

function polar(id: string, ring: Ring, angle: number, r: number, gateway: string | null = null): Placed {
  // r = 0 is written as exact zeros so the centre is never -0.
  const x = r === 0 ? 0 : Math.cos(angle) * r;
  const y = r === 0 ? 0 : Math.sin(angle) * r;
  return { id, ring, angle, r, x, y, gateway };
}

function ring(ids: readonly string[], kind: Ring, r: number, out: Map<string, Placed>): void {
  const ranked = [...ids].sort(byRank);
  ranked.forEach((id, rank) => {
    out.set(id, polar(id, kind, START_ANGLE + (TAU * rank) / ranked.length, r));
  });
}

/** Mean direction of `angles`, or null when they cancel out or there are none. */
function meanAngle(angles: readonly number[]): number | null {
  let x = 0;
  let y = 0;
  for (const a of angles) {
    x += Math.cos(a);
    y += Math.sin(a);
  }
  return Math.hypot(x, y) < 1e-9 ? null : Math.atan2(y, x);
}

/**
 * Places every device. This device sits at the centre. Peers sit on the outer ring at
 * evenly spaced angles ordered by hash rank, so adding a device moves every other one by
 * less than one slot and keeps their order. Relays sit on an inner ring at the mean angle
 * of the devices that use them. Phones hang off their gateway, `satellite` px further out.
 * Above DENSE_OVER devices, online devices move to an inner ring and offline ones stay out.
 */
export function layout(input: LayoutInput, geometry: Geometry): Layout {
  const { self } = input;
  const peers = [...new Set(input.peers)].filter((id) => id !== self);
  const known = new Set(peers);
  const dense = peers.length > DENSE_OVER;
  const nodes = new Map<string, Placed>();
  nodes.set(self, polar(self, 'self', START_ANGLE, 0));

  const relays = new Set<string>();
  for (const id of input.relays?.keys() ?? []) if (known.has(id)) relays.add(id);

  // A satellite hangs off a ring device: never this device (it is the centre), never a
  // missing one, and never another satellite, so chains and cycles fall back to the ring.
  const anchored = (id: string): boolean => {
    const g = input.gateways?.get(id);
    return g === undefined || g === self || g === id || !known.has(g);
  };
  const satellites = new Map<string, string>();
  for (const id of peers) {
    const g = input.gateways?.get(id);
    if (g === undefined || relays.has(id) || anchored(id)) continue;
    if (anchored(g)) satellites.set(id, g);
  }

  const onRing = peers.filter((id) => !relays.has(id) && !satellites.has(id));
  const { radius } = geometry;
  if (dense) {
    const online = input.online ?? new Set<string>();
    ring(onRing.filter((id) => online.has(id)), 'inner', radius * DENSE_ONLINE_RING, nodes);
    ring(onRing.filter((id) => !online.has(id)), 'outer', radius, nodes);
  } else {
    ring(onRing, 'outer', radius, nodes);
  }

  // Relays: the mean angle of their users (a satellite user counts at its gateway).
  const relayR = radius * (dense ? DENSE_RELAY_RING : RELAY_RING);
  const relayAngles = [...relays].sort(byRank).map((id) => {
    const angles: number[] = [];
    for (const user of input.relays?.get(id) ?? []) {
      const at = nodes.get(satellites.get(user) ?? user);
      if (at && at.ring !== 'self' && at.ring !== 'relay') angles.push(at.angle);
    }
    const mean = meanAngle(angles);
    return { id, angle: fromTop(mean ?? START_ANGLE + (TAU * hash(id)) / 2 ** 32) };
  });
  relayAngles.sort((a, b) => a.angle - b.angle || byRank(a.id, b.id));
  const chord = Math.min(1, geometry.spacing / (2 * Math.max(1, relayR)));
  const gap = Math.min(TAU / Math.max(1, relayAngles.length), 2 * Math.asin(chord));
  relayAngles.forEach((relay, i) => {
    const before = relayAngles[i - 1];
    if (before && relay.angle - before.angle < gap) relay.angle = before.angle + gap;
    nodes.set(relay.id, polar(relay.id, 'relay', START_ANGLE + relay.angle, relayR));
  });

  // Satellites: fanned around their gateway's outward direction, in rank order.
  const byGateway = new Map<string, string[]>();
  for (const [id, g] of satellites) byGateway.set(g, [...(byGateway.get(g) ?? []), id]);
  const ratio = Math.min(1, geometry.spacing / (2 * Math.max(1, geometry.satellite)));
  for (const [g, ids] of [...byGateway].sort(([a], [b]) => byRank(a, b))) {
    const at = nodes.get(g);
    if (!at) continue;
    ids.sort(byRank);
    const spread = Math.min(2 * Math.asin(ratio), Math.PI / ids.length);
    ids.forEach((id, i) => {
      const dir = at.angle + (i - (ids.length - 1) / 2) * spread;
      const x = at.x + Math.cos(dir) * geometry.satellite;
      const y = at.y + Math.sin(dir) * geometry.satellite;
      nodes.set(id, polar(id, 'satellite', Math.atan2(y, x), Math.hypot(x, y), g));
    });
  }

  const order = [...nodes.values()]
    .filter((n) => n.ring !== 'self')
    .sort((a, b) => fromTop(a.angle) - fromTop(b.angle) || a.r - b.r || byRank(a.id, b.id))
    .map((n) => n.id);
  return { nodes, order: [self, ...order], dense };
}

/** A position in polar and plain coordinates, as `between` returns it. */
export interface Point {
  readonly angle: number;
  readonly r: number;
  readonly x: number;
  readonly y: number;
}

/**
 * A node part way between two placements: it turns the short way round the centre and
 * moves in or out at the same time, so nobody cuts across the ring. `t` is 0 to 1.
 */
export function between(from: Point, to: Point, t: number): Point {
  const r = from.r + (to.r - from.r) * t;
  const start = from.r === 0 ? to.angle : from.angle;
  const angle = start + turn(start, to.angle) * t;
  return { angle, r, x: r === 0 ? 0 : Math.cos(angle) * r, y: r === 0 ? 0 : Math.sin(angle) * r };
}

/**
 * The progress curve of a CSS easing (`linear` or `cubic-bezier(a, b, c, d)`, as in
 * timing.ts EASING), for motion that CSS cannot run, such as nodes sliding round a ring.
 */
export function curve(css: string): (t: number) => number {
  const m = /^cubic-bezier\(([^)]*)\)$/.exec(css.trim());
  const [x1, y1, x2, y2] = (m?.[1] ?? '').split(',').map(Number);
  if (x1 === undefined || y1 === undefined || x2 === undefined || y2 === undefined || [x1, y1, x2, y2].some(Number.isNaN)) {
    return (t) => Math.min(1, Math.max(0, t));
  }
  const at = (a: number, b: number, u: number): number => 3 * a * u * (1 - u) ** 2 + 3 * b * u * u * (1 - u) + u ** 3;
  return (t) => {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    // Solve x(u) = t by bisection: x is monotonic for 0 ≤ x1, x2 ≤ 1.
    let lo = 0;
    let hi = 1;
    for (let i = 0; i < 30; i += 1) {
      const mid = (lo + hi) / 2;
      if (at(x1, x2, mid) < t) lo = mid;
      else hi = mid;
    }
    return at(y1, y2, (lo + hi) / 2);
  };
}

/** Cuts a name to LABEL_MAX characters, the last one an ellipsis when cut. */
export function truncate(name: string): string {
  const chars = [...name];
  return chars.length <= LABEL_MAX ? name : `${chars.slice(0, LABEL_MAX - 1).join('')}${ELLIPSIS}`;
}
