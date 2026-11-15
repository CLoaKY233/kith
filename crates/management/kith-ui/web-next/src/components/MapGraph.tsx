import { useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as RPointerEvent, type Ref } from 'react';
import type { LinkTick, PathKind, PeerInfo } from '@kith/types';
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
  createHoverCardController,
} from '@ui-sdk/react/hover-card';
import { StatusDot, fmt } from '../ui';

export type MapFilter = 'all' | 'direct' | 'relayed' | 'offline';

export type MapNode = {
  id: string;
  name: string;
  address: string;
  x: number;
  y: number;
  kind: 'me' | 'peer' | 'phone' | 'internet';
  online: boolean;
  path: PathKind | null;
  exit: boolean;
  relay: boolean;
  peer: PeerInfo | null;
  rtt: number | null;
  rate: number | null;
};

export type MapEdge = {
  id: string;
  from: string;
  to: string;
  via: string | null;
  kind: 'direct' | 'relayed' | 'offline' | 'mesh' | 'gateway' | 'internet';
  rate: number | null;
  band: LinkTick['rate_band'];
};

const RING = 270;
/** Offline devices sit on a wider ring so they read apart from the online ones. */
const OUTER = RING + 70;

/** Positions are a pure function of the sorted node ids, so polls never shuffle the map. */
export function layout(
  me: { id: string; name: string },
  peers: PeerInfo[],
  links: LinkTick[],
  relays: Set<string>,
  exitName: string | null,
): { nodes: MapNode[]; edges: MapEdge[] } {
  const mine = new Map(links.filter((l) => l.from === me.id).map((l) => [l.to, l]));
  const phones = peers.filter((p) => p.gateway);
  const ring = peers.filter((p) => !p.gateway).sort((a, b) => a.node.localeCompare(b.node));
  const nodes: MapNode[] = [
    { id: me.id, name: me.name, address: '', x: 0, y: 0, kind: 'me', online: true, path: null, exit: false, relay: false, peer: null, rtt: null, rate: null },
  ];
  const at = new Map<string, MapNode>([[me.id, nodes[0]]]);
  const byName = new Map<string, string>([[me.name, me.id]]);
  const make = (p: PeerInfo, x: number, y: number, kind: MapNode['kind']): MapNode => {
    const l = mine.get(p.node);
    const n: MapNode = {
      id: p.node, name: p.name, address: p.address, x, y, kind,
      online: p.online, path: p.path, exit: p.name === exitName, relay: relays.has(p.node), peer: p,
      rtt: l?.rtt_ms ?? null, rate: l?.rate ?? null,
    };
    nodes.push(n);
    at.set(p.node, n);
    byName.set(p.name, p.node);
    return n;
  };
  ring.forEach((p, i) => {
    const a = (i / Math.max(ring.length, 1)) * Math.PI * 2 - Math.PI / 2;
    const r = p.online ? RING : OUTER;
    make(p, Math.cos(a) * r, Math.sin(a) * r, 'peer');
  });
  const perGateway = new Map<string, number>();
  for (const p of phones.sort((a, b) => a.node.localeCompare(b.node))) {
    const g = at.get(byName.get(p.gateway!) ?? '');
    const k = perGateway.get(p.gateway!) ?? 0;
    perGateway.set(p.gateway!, k + 1);
    const base = g && g.kind !== 'me' ? Math.atan2(g.y, g.x) : Math.PI * 0.75;
    const a = base + (k - 0.5) * 0.35 + 0.17;
    const gx = g?.x ?? 0;
    const gy = g?.y ?? 0;
    const d = g?.kind === 'me' ? 130 : 105;
    make(p, gx + Math.cos(a) * d, gy + Math.sin(a) * d, 'phone');
  }

  const edges: MapEdge[] = [];
  const exitNode = nodes.find((n) => n.exit);
  if (exitNode) {
    const a = exitNode.x === 0 && exitNode.y === 0 ? -Math.PI / 2 : Math.atan2(exitNode.y, exitNode.x);
    const inet: MapNode = {
      id: '__internet', name: 'internet', address: '0.0.0.0/0', x: exitNode.x + Math.cos(a) * 150, y: exitNode.y + Math.sin(a) * 150,
      kind: 'internet', online: true, path: null, exit: false, relay: false, peer: null, rtt: null, rate: null,
    };
    nodes.push(inet);
    edges.push({ id: 'inet', from: exitNode.id, to: inet.id, via: null, kind: 'internet', rate: exitNode.rate, band: null });
  }
  for (const n of nodes) {
    if (!n.peer) continue;
    const l = mine.get(n.id);
    if (n.kind === 'phone') {
      const g = byName.get(n.peer.gateway!) ?? me.id;
      edges.push({ id: `g:${n.id}`, from: g, to: n.id, via: null, kind: n.online ? 'gateway' : 'offline', rate: l?.rate ?? null, band: l?.rate_band ?? null });
      continue;
    }
    const relayed = n.path === 'relay' || n.path === 'stealth';
    const kind = !n.online ? 'offline' : relayed ? 'relayed' : 'direct';
    const via = relayed && l?.relay && at.has(l.relay) ? l.relay : null;
    edges.push({ id: `m:${n.id}`, from: me.id, to: n.id, via, kind, rate: l?.rate ?? null, band: l?.rate_band ?? null });
  }
  const seen = new Set<string>();
  for (const l of links) {
    if (l.from === me.id || l.to === me.id || !at.has(l.from) || !at.has(l.to)) continue;
    const key = [l.from, l.to].sort().join();
    if (seen.has(key)) continue;
    seen.add(key);
    edges.push({ id: `x:${key}`, from: l.from, to: l.to, via: null, kind: 'mesh', rate: l.rate, band: l.rate_band });
  }
  return { nodes, edges };
}

const bandRate = (e: MapEdge) => e.rate ?? (e.band === 'busy' ? 1e6 : e.band === 'light' ? 3e4 : 0);

/** True when the edge carries traffic right now (offline and other devices' links never animate). */
export const edgeLive = (e: MapEdge) => bandRate(e) > 0 && e.kind !== 'offline' && e.kind !== 'mesh';

/** Packets move at a speed proportional to the rate, capped so a busy link stays readable. */
const flowSeconds = (rate: number) => 24 / Math.min(120, Math.max(8, rate / 1e4));

function useReducedMotion() {
  const q = '(prefers-reduced-motion: reduce)';
  const [reduce, setReduce] = useState(() => typeof matchMedia === 'function' && matchMedia(q).matches);
  useEffect(() => {
    const m = matchMedia(q);
    const on = () => setReduce(m.matches);
    m.addEventListener('change', on);
    return () => m.removeEventListener('change', on);
  }, []);
  return reduce;
}

/** Nodes a filter leaves on screen: this device, plus anything a shown edge reaches or bends through. */
export function nodeShown(n: MapNode, edges: MapEdge[], f: MapFilter) {
  return f === 'all' || n.kind === 'me' || edges.some((e) => (e.to === n.id || e.via === n.id) && edgeVisible(e, f));
}

export function edgeVisible(e: MapEdge, f: MapFilter) {
  if (f === 'all') return true;
  if (f === 'direct') return e.kind === 'direct' || e.kind === 'gateway' || e.kind === 'internet';
  if (f === 'relayed') return e.kind === 'relayed';
  return e.kind === 'offline';
}

export type MapControls = { zoom: (factor: number) => void; fit: () => void };
type View = { x: number; y: number; k: number };

const STROKE: Record<MapEdge['kind'], string> = {
  direct: 'var(--color-fg)',
  internet: 'var(--color-fg)',
  relayed: 'var(--color-fg-2)',
  gateway: 'var(--color-fg-2)',
  offline: 'var(--color-fg-3)',
  mesh: 'var(--color-fg-3)',
};
const DASH: Partial<Record<MapEdge['kind'], string>> = { relayed: '6 5', offline: '1 5', internet: '8 3' };

/** Half-size of each mark in screen pixels; marks keep their size at every zoom. */
const SIZE: Record<MapNode['kind'], number> = { me: 12, peer: 8, phone: 6, internet: 14 };
const K_MIN = 0.2;
const K_MAX = 3;
const ZOOM_STEP = 1.6;
const TAP_SLOP = 6;
const DOUBLE_TAP_MS = 300;
const DOUBLE_TAP_SLOP = 24;
const clampK = (k: number) => Math.min(K_MAX, Math.max(K_MIN, k));

const PAD = 32;
const CHAR_W = 7.2;
const LABEL_H = 14;
const LABEL_MAX = 24;
/** Long names are cut on the map; the full name stays in the hover card and the accessible label. */
const shortName = (name: string) => (name.length > LABEL_MAX ? `${name.slice(0, LABEL_MAX - 1)}…` : name);
const PATH_LABEL: Record<PathKind, string> = { direct: 'Direct', relay: 'Relayed', stealth: 'Stealth' };

function fitView(nodes: MapNode[], w: number, h: number): View {
  if (!nodes.length || !w || !h) return { x: w / 2, y: h / 2, k: 1 };
  const xs = nodes.map((n) => n.x);
  const ys = nodes.map((n) => n.y);
  // The offline ring is drawn in full, so it has to fit as well as the marks on it.
  const ring = nodes.some((n) => n.kind === 'peer' && !n.online) ? OUTER : 0;
  const x0 = Math.min(-ring, ...xs);
  const x1 = Math.max(ring, ...xs);
  const y0 = Math.min(-ring, ...ys);
  const y1 = Math.max(ring, ...ys);
  // Labels sit outside the outermost marks on each side; keep them inside the canvas.
  const cap = w * 0.3;
  const side = (pick: (n: MapNode) => boolean) =>
    Math.min(cap, Math.max(0, ...nodes.filter(pick).map((n) => shortName(n.name).length * CHAR_W + SIZE[n.kind] + 10)));
  const mid = (x0 + x1) / 2;
  const left = PAD + side((n) => n.x < mid - (x1 - x0) * 0.25);
  const right = PAD + side((n) => n.x > mid + (x1 - x0) * 0.25);
  const padY = PAD + LABEL_H + 16;
  const k = clampK(Math.min(1.8, (w - left - right) / Math.max(1, x1 - x0), (h - 2 * padY) / Math.max(1, y1 - y0)));
  return { k, x: left + (w - left - right) / 2 - mid * k, y: h / 2 - ((y0 + y1) / 2) * k };
}

/** The middle of the widest gap between angles on a ring, so a label there touches no device. */
function emptiestAngle(angles: number[]): number {
  if (!angles.length) return -Math.PI / 2;
  const sorted = [...angles].sort((a, b) => a - b);
  let best = { gap: -1, at: 0 };
  sorted.forEach((a, i) => {
    const next = sorted[(i + 1) % sorted.length] + (i === sorted.length - 1 ? 2 * Math.PI : 0);
    if (next - a > best.gap) best = { gap: next - a, at: a + (next - a) / 2 };
  });
  return best.at;
}

type Box = { x0: number; y0: number; x1: number; y1: number };
const hits = (a: Box, b: Box) => a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;

/** Label box for text placed at distance d along (cos, sin) from a mark at (x, y). */
function labelBox(x: number, y: number, cos: number, sin: number, d: number, width: number): Box {
  const lx = x + cos * d;
  const ly = y + sin * d;
  const left = cos > 0.35 ? lx : cos < -0.35 ? lx - width : lx - width / 2;
  const top = sin > 0.35 ? ly : sin < -0.35 ? ly - LABEL_H : ly - LABEL_H / 2;
  return { x0: left - 2, y0: top - 1, x1: left + width + 2, y1: top + LABEL_H + 1 };
}

export function MapCanvas({
  nodes,
  edges,
  filter,
  selected,
  onSelect,
  ref,
}: {
  nodes: MapNode[];
  /** Edges to draw; the caller leaves out other devices' links unless asked for. */
  edges: MapEdge[];
  filter: MapFilter;
  selected: string | null;
  onSelect: (id: string | null) => void;
  ref?: Ref<MapControls>;
}) {
  const at = new Map(nodes.map((n) => [n.id, n]));
  const box = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View>({ x: 0, y: 0, k: 1 });
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  /** The gesture in progress: where it started, and whether it has moved past a tap. */
  const gesture = useRef<{ x: number; y: number; moved: boolean; pinch: boolean } | null>(null);
  const lastTap = useRef<{ t: number; x: number; y: number } | null>(null);
  const swallowClick = useRef(false);
  const reduce = useReducedMotion();
  const drawn = useMemo(() => nodes.filter((n) => nodeShown(n, edges, filter)), [nodes, edges, filter]);
  const nodesRef = useRef(drawn);
  nodesRef.current = drawn;

  const fit = useCallback(() => {
    const r = box.current?.getBoundingClientRect();
    if (r) setView(fitView(nodesRef.current, r.width, r.height));
  }, []);
  const zoomAt = useCallback((f: number, px?: number, py?: number) => {
    const r = box.current?.getBoundingClientRect();
    if (!r) return;
    const cx = px ?? r.width / 2;
    const cy = py ?? r.height / 2;
    setView((v) => {
      const k = clampK(v.k * f);
      return { k, x: cx - ((cx - v.x) * k) / v.k, y: cy - ((cy - v.y) * k) / v.k };
    });
  }, []);
  useImperativeHandle(ref, () => ({ zoom: (f) => zoomAt(f), fit }), [zoomAt, fit]);

  useEffect(() => {
    const el = box.current!;
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    // Only a pinch (trackpads send it as ctrl+wheel) zooms; a plain scroll keeps scrolling the page.
    const onWheel = (ev: WheelEvent) => {
      if (!ev.ctrlKey) return;
      ev.preventDefault();
      const r = el.getBoundingClientRect();
      zoomAt(Math.exp(-ev.deltaY * (ev.deltaMode === 1 ? 0.03 : 0.0015)), ev.clientX - r.left, ev.clientY - r.top);
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      ro.disconnect();
      el.removeEventListener('wheel', onWheel);
    };
  }, [fit, zoomAt]);
  useLayoutEffect(() => {
    fit();
    // The canvas may still be settling into its height on the first frame; fit once more.
    const frame = requestAnimationFrame(fit);
    return () => cancelAnimationFrame(frame);
  }, [drawn.length, fit]);

  const local = (x: number, y: number) => {
    const r = box.current!.getBoundingClientRect();
    return { x: x - r.left, y: y - r.top };
  };
  const pair = () => {
    const [a, b] = [...pointers.current.values()];
    return { mx: (a.x + b.x) / 2, my: (a.y + b.y) / 2, d: Math.hypot(a.x - b.x, a.y - b.y) };
  };
  const down = (e: RPointerEvent) => {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.current.size === 1) {
      gesture.current = { x: e.clientX, y: e.clientY, moved: false, pinch: false };
    } else if (gesture.current) {
      gesture.current.moved = true;
      gesture.current.pinch = true;
      for (const id of pointers.current.keys()) {
        if (!box.current!.hasPointerCapture(id)) box.current!.setPointerCapture(id);
      }
    }
  };
  const move = (e: RPointerEvent) => {
    const p = pointers.current.get(e.pointerId);
    const g = gesture.current;
    if (!p || !g) return;
    if (pointers.current.size >= 2) {
      const before = pair();
      p.x = e.clientX;
      p.y = e.clientY;
      const after = pair();
      const f = before.d > 0 ? after.d / before.d : 1;
      const c = local(after.mx, after.my);
      const dx = after.mx - before.mx;
      const dy = after.my - before.my;
      setView((v) => {
        const k = clampK(v.k * f);
        return { k, x: c.x - ((c.x - dx - v.x) * k) / v.k, y: c.y - ((c.y - dy - v.y) * k) / v.k };
      });
      return;
    }
    const dx = e.clientX - p.x;
    const dy = e.clientY - p.y;
    p.x = e.clientX;
    p.y = e.clientY;
    if (!g.moved) {
      if (Math.hypot(e.clientX - g.x, e.clientY - g.y) < TAP_SLOP) return;
      g.moved = true;
      // Capture only once it is a drag, so a tap on a device still reaches its link.
      box.current!.setPointerCapture(e.pointerId);
      setView((v) => ({ ...v, x: v.x + e.clientX - g.x, y: v.y + e.clientY - g.y }));
      return;
    }
    if (g.pinch) return;
    setView((v) => ({ ...v, x: v.x + dx, y: v.y + dy }));
  };
  const up = (e: RPointerEvent) => {
    if (!pointers.current.delete(e.pointerId)) return;
    const g = gesture.current;
    if (pointers.current.size > 0) return;
    gesture.current = null;
    if (!g) return;
    if (g.moved) {
      swallowClick.current = true;
      lastTap.current = null;
      return;
    }
    const onNode = !!(e.target as Element).closest('a');
    const prev = lastTap.current;
    if (prev && e.timeStamp - prev.t < DOUBLE_TAP_MS && Math.hypot(e.clientX - prev.x, e.clientY - prev.y) < DOUBLE_TAP_SLOP) {
      lastTap.current = null;
      const c = local(e.clientX, e.clientY);
      zoomAt(ZOOM_STEP, c.x, c.y);
      return;
    }
    lastTap.current = { t: e.timeStamp, x: e.clientX, y: e.clientY };
    if (!onNode) onSelect(null);
  };
  const cancel = (e: RPointerEvent) => {
    pointers.current.delete(e.pointerId);
    if (pointers.current.size === 0) gesture.current = null;
  };

  const sx = (x: number) => view.x + x * view.k;
  const sy = (y: number) => view.y + y * view.k;
  const pathFor = (e: MapEdge) => {
    const a = at.get(e.from);
    const b = at.get(e.to);
    if (!a || !b) return null;
    const r = e.via ? at.get(e.via) : undefined;
    if (r) {
      // Bend through the relay: the control point pulls the curve onto it.
      return `M${sx(a.x)},${sy(a.y)} Q${sx(2 * r.x - (a.x + b.x) / 2)},${sy(2 * r.y - (a.y + b.y) / 2)} ${sx(b.x)},${sy(b.y)}`;
    }
    if (e.kind === 'mesh') {
      return `M${sx(a.x)},${sy(a.y)} Q${sx(((a.x + b.x) / 2) * 0.7)},${sy(((a.y + b.y) / 2) * 0.7)} ${sx(b.x)},${sy(b.y)}`;
    }
    return `M${sx(a.x)},${sy(a.y)} L${sx(b.x)},${sy(b.y)}`;
  };

  // Labels go outside each mark, away from the centre (or the phone's gateway), then step further
  // out along that direction until they clear every mark and every label already placed. Overlap is
  // unaffected by panning, so this is laid out around the origin and only redone when the zoom changes.
  const offline = drawn.filter((n) => n.kind === 'peer' && !n.online);
  const ringLabelAt = emptiestAngle(drawn.filter((n) => n.kind !== 'me').map((n) => Math.atan2(n.y, n.x)));
  const labels = useMemo(() => {
    const k = view.k;
    const marks: Box[] = drawn.map((n) => {
      const g = (n.exit || n.relay ? SIZE[n.kind] + 5 : SIZE[n.kind]) + 2;
      return { x0: n.x * k - g, y0: n.y * k - g, x1: n.x * k + g, y1: n.y * k + g };
    });
    if (offline.length) {
      // The ring's own "Offline" label is an obstacle too, so no device name lands on it.
      const rx = Math.cos(ringLabelAt) * OUTER * k;
      const ry = Math.sin(ringLabelAt) * OUTER * k;
      marks.push({ x0: rx - 34, y0: ry - 9, x1: rx + 34, y1: ry + 9 });
    }
    const placed: Box[] = [];
    const out = new Map<string, { angle: number; d: number; text: boolean }>();
    const clear = (b: Box) => !marks.some((m) => hits(m, b)) && !placed.some((m) => hits(m, b));
    for (const n of [...drawn].sort((a, b) => (a.kind === 'me' ? -1 : b.kind === 'me' ? 1 : 0))) {
      const gateway = n.kind === 'phone' && n.peer?.gateway ? nodes.find((g) => g.name === n.peer!.gateway) : undefined;
      const angle = n.kind === 'me' ? Math.PI / 2 : gateway ? Math.atan2(n.y - gateway.y, n.x - gateway.x) : Math.atan2(n.y, n.x);
      const cos = Math.cos(angle);
      const sin = Math.sin(angle);
      const width = shortName(n.name).length * CHAR_W;
      let d = (n.exit || n.relay ? SIZE[n.kind] + 5 : SIZE[n.kind]) + 8;
      let ang = angle;
      let b = labelBox(n.x * k, n.y * k, cos, sin, d, width);
      let ok = clear(b);
      // Prefer sliding the label round its mark over pushing it away, so it stays beside it.
      search: for (let i = 0; i < 12 && !clear(b); i++) {
        for (const turn of [0, 0.6, -0.6, 1.2, -1.2, Math.PI]) {
          const c = Math.cos(angle + turn);
          const s2 = Math.sin(angle + turn);
          const t = labelBox(n.x * k, n.y * k, c, s2, d, width);
          if (clear(t)) {
            b = t;
            ang = angle + turn;
            ok = true;
            break search;
          }
        }
        d += 3;
      }
      // A name that can't find room stays off the map (zoom in, hover or select to read it)
      // rather than printing over another one.
      if (ok || n.kind === 'me') {
        placed.push(b);
        out.set(n.id, { angle: ang, d, text: true });
      } else {
        out.set(n.id, { angle, d: (n.exit || n.relay ? SIZE[n.kind] + 5 : SIZE[n.kind]) + 8, text: false });
      }
    }
    return out;
  }, [drawn, nodes, view.k, offline.length, ringLabelAt]);

  const focus = selected && at.has(selected)
    ? new Set([selected, ...edges.filter((e) => e.from === selected || e.to === selected || e.via === selected).flatMap((e) => [e.from, e.to])])
    : null;

  return (
    <div
      ref={box}
      className="absolute inset-0 cursor-grab touch-none select-none overflow-hidden active:cursor-grabbing"
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={cancel}
      onLostPointerCapture={cancel}
      onClickCapture={(e) => {
        // A drag that began on a device must not also open it.
        if (swallowClick.current) {
          swallowClick.current = false;
          e.preventDefault();
          e.stopPropagation();
        }
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && selected) onSelect(null);
        else if (e.key === '+' || e.key === '=') zoomAt(ZOOM_STEP);
        else if (e.key === '-' || e.key === '_') zoomAt(1 / ZOOM_STEP);
        else if (e.key === '0') fit();
        else return;
        e.preventDefault();
      }}
    >
      <svg className="h-full w-full" role="group" aria-label="Network map">
        {offline.length > 0 && (
          <g aria-hidden="true">
            <circle cx={view.x} cy={view.y} r={OUTER * view.k} fill="none" stroke="var(--color-line)" strokeDasharray="2 4" />
            <text
              x={view.x + Math.cos(ringLabelAt) * OUTER * view.k}
              y={view.y + Math.sin(ringLabelAt) * OUTER * view.k}
              textAnchor="middle"
              dominantBaseline="middle"
              fontSize={11}
              fill="var(--color-fg-3)"
              stroke="var(--color-ink-0)"
              strokeWidth={4}
              paintOrder="stroke"
              className="font-mono uppercase tracking-[0.08em]"
            >
              Offline
            </text>
          </g>
        )}
        {edges.map((e) => {
          const shown = edgeVisible(e, filter);
          const dim = focus && !(focus.has(e.from) && focus.has(e.to));
          const d = pathFor(e);
          if (!d) return null;
          const r = bandRate(e);
          const live = shown && edgeLive(e);
          const w = e.kind === 'mesh' || e.kind === 'offline' ? 1 : Math.min(3, 1 + Math.log10(1 + r) * 0.3);
          const op = !shown ? 0.05 : dim ? 0.12 : e.kind === 'offline' ? 0.4 : e.kind === 'mesh' ? 0.3 : 0.85;
          return (
            <g key={e.id} style={{ opacity: op, transition: 'opacity 200ms ease-out' }}>
              <path d={d} fill="none" stroke={STROKE[e.kind]} strokeWidth={w} strokeOpacity={live ? 0.35 : 0.7} strokeDasharray={DASH[e.kind]} />
              {live && (
                <path
                  d={d}
                  className={reduce ? undefined : 'flow'}
                  fill="none"
                  stroke={STROKE[e.kind]}
                  strokeWidth={w + 1}
                  strokeDasharray="2 10"
                  style={reduce ? undefined : { animationDuration: `${flowSeconds(r)}s` }}
                />
              )}
            </g>
          );
        })}
        {nodes.map((n) => {
          const l = labels.get(n.id);
          return (
            <NodeMark
              key={n.id}
              n={n}
              x={sx(n.x)}
              y={sy(n.y)}
              angle={l?.angle ?? Math.atan2(n.y, n.x)}
              gap={l?.d ?? SIZE[n.kind] + 8}
              label={!!l?.text}
              selected={selected === n.id}
              dim={!!focus && !focus.has(n.id)}
              hidden={!l}
              onSelect={onSelect}
            />
          );
        })}
      </svg>
      <HoverCard controller={hover} delay={250} closeDelay={120}>
        <HoverCardContent<MapNode> side="top" sideOffset={10} className="pointer-events-none w-56">
          {(s) => s.payload && <HoverBody n={s.payload} />}
        </HoverCardContent>
      </HoverCard>
    </div>
  );
}

const hover = createHoverCardController<MapNode>();

function HoverBody({ n }: { n: MapNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <StatusDot status={n.online ? 'online' : 'offline'} />
        <span className="min-w-0 truncate text-[14px] text-fg" title={n.name}>{n.name}</span>
      </div>
      <div className="truncate font-mono text-[12px] text-fg-3">
        {n.kind === 'me' ? 'This device' : n.kind === 'internet' ? 'Reached through the exit' : n.address}
      </div>
      {n.peer && (
        <div className="grid grid-cols-3 gap-2 pt-1 font-mono text-[12px] tabular-nums">
          <span className="truncate text-fg-3">{!n.online ? 'Offline' : n.path ? PATH_LABEL[n.path] : 'Online'}</span>
          <span className="truncate text-right text-fg-2">{n.rtt != null ? fmt.ms(n.rtt) : '—'}</span>
          <span className="truncate text-right text-fg">{n.rate != null ? fmt.rate(n.rate) : '—'}</span>
        </div>
      )}
    </div>
  );
}

function NodeMark({
  n,
  x,
  y,
  angle,
  gap,
  selected,
  dim,
  hidden,
  label,
  onSelect,
}: {
  n: MapNode;
  x: number;
  y: number;
  angle: number;
  gap: number;
  selected: boolean;
  dim: boolean;
  hidden: boolean;
  label: boolean;
  onSelect: (id: string) => void;
}) {
  const r = SIZE[n.kind];
  const ring = n.exit || n.relay ? r + 5 : r;
  const color = !n.online
    ? 'var(--color-fg-3)'
    : n.kind === 'me' || n.kind === 'internet' || n.exit || n.path === 'direct'
      ? 'var(--color-fg)'
      : 'var(--color-fg-2)';
  const [hot, setHot] = useState(false);
  const [focused, setFocused] = useState(false);
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const anchor = cos > 0.35 ? 'start' : cos < -0.35 ? 'end' : 'middle';
  const baseline = sin > 0.35 ? 'hanging' : sin < -0.35 ? 'auto' : 'central';
  const tag = [n.exit && 'exit', n.relay && 'relay'].filter(Boolean).join(' · ');
  return (
    <HoverCardTrigger
      controller={hover}
      payload={n}
      href={n.peer ? `/devices/${encodeURIComponent(n.name)}` : '#'}
      onClick={(e) => {
        e.preventDefault();
        onSelect(n.id);
      }}
      aria-label={`${n.name}, ${n.online ? 'online' : 'offline'}${tag ? `, ${tag}` : ''}`}
      tabIndex={hidden ? -1 : undefined}
      aria-hidden={hidden || undefined}
      style={{ opacity: hidden ? 0.08 : dim ? 0.3 : 1, transition: 'opacity 200ms ease-out', cursor: 'pointer', outline: 'none', pointerEvents: hidden ? 'none' : undefined }}
      className="map-node"
      onKeyDown={(e) => {
        if (e.key === ' ') {
          e.preventDefault();
          onSelect(n.id);
        }
      }}
      onPointerEnter={() => setHot(true)}
      onPointerLeave={() => setHot(false)}
      onFocus={(e) => {
        setHot(true);
        setFocused(e.currentTarget.matches(':focus-visible'));
      }}
      onBlur={() => {
        setHot(false);
        setFocused(false);
      }}
    >
      <g transform={`translate(${x},${y})`}>
        <rect x={-ring - 6} y={-ring - 6} width={2 * ring + 12} height={2 * ring + 12} fill="transparent" />
        {n.exit && <rect x={-ring} y={-ring} width={2 * ring} height={2 * ring} fill="none" stroke="var(--color-fg)" strokeDasharray="4 2" />}
        {n.relay && !n.exit && <rect x={-ring} y={-ring} width={2 * ring} height={2 * ring} fill="none" stroke="var(--color-fg-2)" transform="rotate(45)" />}
        {(selected || focused) && (
          <rect x={-ring - 4} y={-ring - 4} width={2 * ring + 8} height={2 * ring + 8} fill="none" stroke="var(--color-fg)" strokeWidth={1.5} />
        )}
        {n.kind === 'internet' ? (
          <path
            d="M-30,10 a14,14 0 0 1 4,-24 a18,18 0 0 1 32,-6 a14,14 0 0 1 22,14 a11,11 0 0 1 -2,22 z"
            transform="scale(0.5) translate(2,2)"
            fill="var(--color-ink-0)"
            stroke={color}
            strokeWidth={2}
          />
        ) : (
          <rect
            x={-r}
            y={-r}
            width={2 * r}
            height={2 * r}
            fill="var(--color-ink-0)"
            stroke={color}
            strokeWidth={n.kind === 'me' ? 2 : 1}
            strokeDasharray={n.online ? undefined : '2 2'}
          />
        )}
        {n.online && n.kind !== 'internet' && <rect x={-r / 3} y={-r / 3} width={(2 * r) / 3} height={(2 * r) / 3} fill={color} />}
        {(label || hot || selected) && (
        <text
          x={cos * gap}
          y={sin * gap}
          textAnchor={anchor}
          dominantBaseline={baseline}
          fontSize={12}
          fill={hot || selected ? 'var(--color-fg)' : 'var(--color-fg-2)'}
          stroke="var(--color-ink-0)"
          strokeWidth={3}
          paintOrder="stroke"
          strokeLinejoin="round"
          className="font-mono"
        >
          <title>{n.name}</title>
          {shortName(n.name)}
        </text>
        )}
      </g>
    </HoverCardTrigger>
  );
}
