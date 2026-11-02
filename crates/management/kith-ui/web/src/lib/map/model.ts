// What the map draws, worked out from the store's data (spec §9.2): the devices of one
// network, the layout input, the edges with their style, node labels and the MapTable
// rows. Pure functions, so Map.svelte only renders and animates.

import { copy, DOT } from '../copy';
import { ago, rtt } from '../format';
import { IDLE_BYTES_PER_SEC, QUIET_HANDSHAKE_MS } from '../live/timing';
import type { ConnState } from '../types';
import type { LinkTick, PathKind, PeerInfo, RateBand, StatusView } from '../types.gen';
import { truncate, type LayoutInput } from './layout';

/** Connection states in which the map's values are last known, not live (§8.5). */
export const LAST_KNOWN: ReadonlySet<ConnState> = new Set(['stale', 'daemon_down', 'ui_gone', 'unauth']);

/** An edge turns amber above this round-trip time, in ms. */
export const RTT_WARN_MS = 150;
/** An edge turns amber above this share of lost probes. */
export const LOSS_WARN = 0.02;
/** Edge widths in px, on a square-root scale of rate. */
export const EDGE_MIN = 1.5;
export const EDGE_MAX = 3.5;
const BAND_WIDTH: Record<RateBand, number> = {
  idle: EDGE_MIN,
  light: (EDGE_MIN + EDGE_MAX) / 2,
  busy: EDGE_MAX,
};

export interface MapDevice {
  readonly id: string;
  readonly name: string;
  readonly self: boolean;
  readonly online: boolean;
  readonly address: string;
  /** The daemon's view of it; null for this device. */
  readonly peer: PeerInfo | null;
  /** The id of the device it connects through, for phones. */
  readonly gateway: string | null;
}

export interface MapModel {
  readonly net: string;
  readonly networkName: string;
  readonly self: string;
  /** Every device, this one included, by id. */
  readonly devices: ReadonlyMap<string, MapDevice>;
  readonly links: readonly LinkTick[];
}

/** Finds a network by id, or by name when the URL uses the name (§4.1). */
export function resolveNetwork(status: StatusView | null, net: string): string | null {
  const networks = status?.networks ?? [];
  return (networks.find((n) => n.id === net) ?? networks.find((n) => n.name === net))?.id ?? null;
}

/** The devices and links of one network, or null until the status names this device. */
export function buildModel(
  status: StatusView | null,
  peers: Readonly<Record<string, PeerInfo>> | undefined,
  links: readonly LinkTick[] | undefined,
  net: string,
): MapModel | null {
  const self = status?.node;
  const network = status?.networks.find((n) => n.id === net);
  if (!status || !self || !network) return null;
  const me = status.memberships.find((m) => m.network === net);
  const list = Object.values(peers ?? {});
  const idByName = new Map(list.map((p) => [p.name, p.node]));
  if (me) idByName.set(me.name, self);
  const devices = new Map<string, MapDevice>();
  devices.set(self, {
    id: self,
    name: me?.name ?? status.host.name,
    self: true,
    online: true,
    address: me?.address ?? '',
    peer: null,
    gateway: null,
  });
  for (const p of list) {
    if (p.node === self) continue;
    devices.set(p.node, {
      id: p.node,
      name: p.name,
      self: false,
      online: p.online,
      address: p.address,
      peer: p,
      gateway: p.gateway === null ? null : (idByName.get(p.gateway) ?? null),
    });
  }
  return { net, networkName: network.name, self, devices, links: links ?? [] };
}

/** Relay id → the devices whose reported path goes through it. */
export function relayUsers(model: MapModel): Map<string, Set<string>> {
  const out = new Map<string, Set<string>>();
  for (const link of model.links) {
    const relay = link.relay;
    if (link.path === 'direct' || !relay || relay === model.self || !model.devices.has(relay)) continue;
    const users = out.get(relay) ?? new Set<string>();
    for (const id of [link.from, link.to]) {
      if (id !== model.self && id !== relay && model.devices.has(id)) users.add(id);
    }
    out.set(relay, users);
  }
  return out;
}

/**
 * Adds `fresh` relay use to `memory` and returns the result; the same object when nothing
 * is new. Relays are remembered for the life of the page so a path change does not move
 * a relay across the map (§3.5: map nodes move only when the node set changes).
 */
export function remember(
  memory: ReadonlyMap<string, ReadonlySet<string>>,
  fresh: ReadonlyMap<string, ReadonlySet<string>>,
): ReadonlyMap<string, ReadonlySet<string>> {
  let next: Map<string, ReadonlySet<string>> | null = null;
  for (const [relay, users] of fresh) {
    const known = memory.get(relay);
    const missing = [...users].filter((u) => !known?.has(u));
    if (known && missing.length === 0) continue;
    next ??= new Map(memory);
    next.set(relay, new Set([...(known ?? []), ...missing]));
  }
  return next ?? memory;
}

/** The layout input for a model; relays come from the remembered use. */
export function layoutInput(
  model: MapModel,
  relays: ReadonlyMap<string, ReadonlySet<string>>,
): LayoutInput {
  const gateways = new Map<string, string>();
  const online = new Set<string>();
  const peers: string[] = [];
  for (const d of model.devices.values()) {
    if (d.self) continue;
    peers.push(d.id);
    if (d.gateway) gateways.set(d.id, d.gateway);
    if (d.online) online.add(d.id);
  }
  const relayList = new Map<string, string[]>();
  for (const [relay, users] of relays) {
    if (model.devices.has(relay)) relayList.set(relay, [...users].filter((u) => model.devices.has(u)));
  }
  return { self: model.self, peers, relays: relayList, gateways, online };
}

/**
 * A string that changes exactly when the layout would. `online` counts only when it
 * places nodes (above DENSE_OVER devices).
 */
export function layoutKey(input: LayoutInput, dense: boolean): string {
  const sorted = (values: Iterable<string>): string => [...values].sort().join(',');
  const relays = [...(input.relays ?? new Map<string, readonly string[]>())]
    .map(([id, users]) => `${id}<${sorted(users)}`)
    .sort();
  const gateways = [...(input.gateways ?? new Map<string, string>())].map(([id, g]) => `${id}>${g}`).sort();
  return [
    input.self,
    sorted(input.peers),
    relays.join(';'),
    gateways.join(';'),
    dense ? sorted(input.online ?? []) : '',
  ].join('|');
}

/** The path as words: `direct`, `via vps`, `stealth via vps`. */
export function pathText(path: PathKind | null, relayName: string | null): string {
  if (path === null) return copy.units.none;
  if (path === 'direct') return copy.path.direct;
  const relay = relayName ?? copy.units.none;
  return path === 'relay' ? copy.path.via(relay) : copy.path.stealthVia(relay);
}

export type EdgeKind = 'direct' | 'relayed' | 'gateway';
export type Tone = 'neutral' | 'attn' | 'danger';

export interface MapEdge {
  readonly key: string;
  readonly kind: EdgeKind;
  /** Ends, and the relay a relayed edge is drawn through. */
  readonly a: string;
  readonly b: string;
  readonly via: string | null;
  /** The report behind it; null for a gateway edge nobody reported. */
  readonly link: LinkTick | null;
  readonly width: number;
  readonly tone: Tone;
  /** Drawn at full strength: no device is focused, or it touches the focused one. */
  readonly strong: boolean;
  /** Shows its RTT: it touches the focused device. */
  readonly labelled: boolean;
}

/** Width in px: a square-root scale of rate, or the band members get (§9.2). */
export function edgeWidth(link: LinkTick | null, maxRate: number): number {
  if (!link) return EDGE_MIN;
  if (link.rate !== null) {
    if (link.rate < IDLE_BYTES_PER_SEC || maxRate <= 0) return EDGE_MIN;
    return EDGE_MIN + (EDGE_MAX - EDGE_MIN) * Math.sqrt(Math.min(1, link.rate / maxRate));
  }
  return link.rate_band ? BAND_WIDTH[link.rate_band] : EDGE_MIN;
}

/**
 * Red when this device's link reaches a device reported online whose handshake is older
 * than 3 min; amber above RTT_WARN_MS or LOSS_WARN; otherwise neutral.
 */
export function edgeTone(link: LinkTick | null, model: MapModel): Tone {
  if (!link) return 'neutral';
  const peer = model.devices.get(link.to)?.peer;
  const seen = peer?.last_seen_secs;
  if (link.from === model.self && peer?.online && seen != null && seen * 1000 > QUIET_HANDSHAKE_MS) {
    return 'danger';
  }
  if ((link.rtt_ms ?? 0) > RTT_WARN_MS || (link.loss ?? 0) > LOSS_WARN) return 'attn';
  return 'neutral';
}

function shape(link: LinkTick, model: MapModel): { kind: EdgeKind; a: string; b: string; via: string | null } {
  const from = model.devices.get(link.from);
  const to = model.devices.get(link.to);
  // A phone's link is drawn as its hanging line to the gateway.
  for (const phone of [to, from]) {
    if (phone?.gateway && model.devices.has(phone.gateway)) {
      return { kind: 'gateway', a: phone.gateway, b: phone.id, via: null };
    }
  }
  const relay = link.relay;
  if (link.path !== 'direct' && relay && relay !== link.from && relay !== link.to && model.devices.has(relay)) {
    return { kind: 'relayed', a: link.from, b: link.to, via: relay };
  }
  return { kind: 'direct', a: link.from, b: link.to, via: null };
}

function edgeKey(kind: EdgeKind, a: string, b: string, via: string | null): string {
  return `${kind}:${a < b ? `${a}:${b}` : `${b}:${a}`}:${via ?? ''}`;
}

/**
 * The edges to draw. By default only this device's own links; with `showAll`, every
 * report. Focusing a device adds its reported links at full strength and dims the rest
 * (nothing dims when it has no links, such as an offline device).
 * Phones always hang off their gateway.
 */
export function edges(model: MapModel, options: { showAll: boolean; focus: string | null }): MapEdge[] {
  const focus = options.focus !== null && options.focus !== model.self && model.devices.has(options.focus)
    ? options.focus
    : null;
  const touches = (link: LinkTick): boolean => focus !== null && (link.from === focus || link.to === focus);
  const maxRate = model.links.reduce((max, l) => Math.max(max, l.rate ?? 0), 0);
  const rank = (link: LinkTick): number =>
    (link.from === model.self ? 0 : link.from === focus ? 1 : 2) * 1e6 + link.age_secs;

  const chosen = new Map<string, { link: LinkTick | null; kind: EdgeKind; a: string; b: string; via: string | null }>();
  const visible = model.links
    .filter((l) => model.devices.has(l.from) && model.devices.has(l.to))
    .filter((l) => options.showAll || l.from === model.self || touches(l))
    .sort((x, y) => rank(x) - rank(y));
  for (const link of visible) {
    const s = shape(link, model);
    const key = edgeKey(s.kind, s.a, s.b, s.via);
    if (!chosen.has(key)) chosen.set(key, { link, ...s });
  }
  // A relayed edge draws its far leg (relay to peer) dashed; a direct report of that leg
  // would sit on top of the dashes and hide the relaying. This device's own link to the
  // relay stays: it is a real link with its own width.
  const legs = new Set<string>();
  for (const e of chosen.values()) {
    if (e.kind !== 'relayed' || !e.via) continue;
    for (const end of [e.a, e.b]) {
      if (end !== model.self) legs.add(edgeKey('direct', end, e.via, null));
    }
  }
  for (const key of legs) chosen.delete(key);
  for (const d of model.devices.values()) {
    if (!d.gateway || !model.devices.has(d.gateway)) continue;
    const key = edgeKey('gateway', d.gateway, d.id, null);
    if (!chosen.has(key)) chosen.set(key, { link: null, kind: 'gateway', a: d.gateway, b: d.id, via: null });
  }

  const isNear = (e: { a: string; b: string; link: LinkTick | null }): boolean =>
    focus !== null && (e.a === focus || e.b === focus || (e.link !== null && touches(e.link)));
  // A focused device with no links to show dims nothing: there is nothing to bring forward.
  const dims = [...chosen.values()].some(isNear);
  return [...chosen.entries()]
    .map(([key, e]) => {
      const near = isNear(e);
      return {
        key,
        kind: e.kind,
        a: e.a,
        b: e.b,
        via: e.via,
        link: e.link,
        width: edgeWidth(e.link, maxRate),
        tone: edgeTone(e.link, model),
        strong: !dims || near,
        labelled: near && e.link?.rtt_ms != null,
      };
    })
    .sort((x, y) => Number(x.strong) - Number(y.strong) || (x.key < y.key ? -1 : 1));
}

/** `ago` for a device last seen `secs` ago, or `not connected yet`. */
function seen(peer: PeerInfo | null): string {
  return peer?.last_seen_secs == null ? copy.device.notConnectedYet : ago(peer.last_seen_secs);
}

/** A device is waiting for its first connection: never seen at all (§5 DeviceRow). */
export function waiting(device: MapDevice): boolean {
  return !device.self && !device.online && device.peer?.last_seen_secs == null;
}

/** The label beside a node: `this Mac`, `nas`, or `laptop · 2 h ago` when offline. */
export function nodeLabel(device: MapDevice, mac: boolean): string {
  if (device.self) return mac ? copy.device.thisMac : copy.device.thisDevice;
  return device.online ? truncate(device.name) : copy.map.offlineLabel(truncate(device.name), seen(device.peer));
}

/** The link this device reports to `id`, if any. */
export function ownLink(model: MapModel, id: string): LinkTick | null {
  return model.links.find((l) => l.from === model.self && l.to === id) ?? null;
}

/** The freshest report about the link to `id`: its own report first, then this device's. */
export function report(model: MapModel, id: string): LinkTick | null {
  return (
    model.links.find((l) => l.from === id && l.to === model.self) ??
    ownLink(model, id) ??
    model.links.find((l) => l.to === id) ??
    null
  );
}

/** The path in words for a peer: through its gateway, or this device's reported path. */
export function devicePath(model: MapModel, device: MapDevice): string {
  const gateway = device.gateway ? model.devices.get(device.gateway) : null;
  if (gateway) return copy.path.via(gateway.self ? copy.device.thisDevice : gateway.name);
  const link = ownLink(model, device.id);
  const relay = link?.relay ? (model.devices.get(link.relay)?.name ?? null) : null;
  return pathText(link?.path ?? device.peer?.path ?? null, relay);
}

/** The word that carries an edge's colour (§11.5), or '' when neutral. */
export function toneWord(tone: Tone): string {
  return tone === 'attn' ? copy.map.slow : tone === 'danger' ? copy.map.unreachable : '';
}

/** The node's accessible name: "laptop, online, direct, 23 milliseconds". */
export function nodeName(model: MapModel, device: MapDevice, stale: boolean): string {
  if (device.self) return copy.map.nodeLabel([device.name, copy.device.thisDevice]);
  if (!device.online) {
    return copy.map.nodeLabel([device.name, copy.status.offline, copy.device.seenAgo(seen(device.peer))]);
  }
  const link = ownLink(model, device.id);
  const ms = stale ? null : (link?.rtt_ms ?? null);
  return copy.map.nodeLabel([
    device.name,
    stale ? copy.status.stale : copy.status.online,
    devicePath(model, device),
    ms === null ? '' : copy.spoken.milliseconds(Math.round(ms)),
    stale ? '' : toneWord(edgeTone(link, model)),
  ]);
}

export interface TableRow {
  readonly key: string;
  readonly from: string;
  readonly to: string;
  readonly toId: string;
  readonly path: string;
  readonly latency: string;
}

/**
 * MapTable rows (§9.2): every reported link as from · to · path · latency, this device's
 * own first, then a row for each device nobody reports, so every device appears.
 */
export function tableRows(model: MapModel, stale: boolean): TableRow[] {
  const name = (id: string): string => model.devices.get(id)?.name ?? id;
  const rows: TableRow[] = [];
  const reached = new Set<string>();
  const links = model.links
    .filter((l) => model.devices.has(l.from) && model.devices.has(l.to))
    .sort((x, y) => {
      const own = Number(y.from === model.self) - Number(x.from === model.self);
      return own || name(x.from).localeCompare(name(y.from)) || name(x.to).localeCompare(name(y.to));
    });
  for (const l of links) {
    reached.add(l.to);
    reached.add(l.from);
    rows.push({
      key: `${l.from}>${l.to}`,
      from: name(l.from),
      to: name(l.to),
      toId: l.to,
      path: pathText(l.path, l.relay ? name(l.relay) : null),
      latency: [rtt(stale ? null : l.rtt_ms), stale ? '' : toneWord(edgeTone(l, model))]
        .filter(Boolean)
        .join(DOT),
    });
  }
  const self = name(model.self);
  const quiet = [...model.devices.values()]
    .filter((d) => !d.self && !reached.has(d.id))
    .sort((x, y) => x.name.localeCompare(y.name));
  for (const d of quiet) {
    rows.push({
      key: `${model.self}>${d.id}`,
      from: self,
      to: d.name,
      toId: d.id,
      path: d.online ? pathText(d.peer?.path ?? null, null) : copy.status.offline,
      latency: copy.units.none,
    });
  }
  return rows;
}
