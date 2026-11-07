// What the Devices list and the Device detail derive from the store (spec §6.3, §6.4,
// §12.4): resolving the URL's names, filters and counts, the verdict sentence, a device's
// history and names, and window totals. Pure, so the tests run it on the office fixture.

import { copy, DOT } from '../copy';
import { ago, rtt as formatRtt } from '../format';
import {
  FLAP_COUNT,
  FLAP_WINDOW_MS,
  IDLE_BYTES_PER_SEC,
  LONG_GONE_MS,
  QUIET_HANDSHAKE_MS,
  RECENTLY_SEEN_MS,
} from '../live/timing';
import type { DevicesFilter, TrafficColumns } from '../types';
import type {
  BlockedGroup,
  Entry,
  Membership,
  NetworkSummary,
  PathKind,
  PeerInfo,
  RecordView,
  StatusView,
} from '../types.gen';

/** One row of the Devices list: a device of the network, or this device. */
export type DeviceItem = {
  readonly peer: PeerInfo;
  /** `net/node`, the key into live.numbers, live.traffic and live.changed. */
  readonly key: string;
  readonly self: boolean;
};

/** Finds the network the URL names: by name, then by id or a short id prefix (§4.1). */
export function resolveNetwork(status: StatusView | null, param: string): NetworkSummary | null {
  if (!status) return null;
  return (
    status.networks.find((n) => n.name === param) ??
    status.networks.find((n) => n.id === param || (param.length >= 6 && n.id.startsWith(param))) ??
    null
  );
}

/** This device's standing in a network. */
export function membershipOf(status: StatusView | null, netId: string): Membership | null {
  return status?.memberships.find((m) => m.network === netId) ?? null;
}

/** This device as a row: the peers list never includes it. */
export function selfPeer(status: StatusView | null, membership: Membership | null): PeerInfo | null {
  if (!status?.node || !membership) return null;
  return {
    node: status.node,
    invite: null,
    settings: null,
    via_this_exit: false,
    name: membership.name,
    address: membership.address,
    path: null,
    path_reason: null,
    last_seen_secs: 0,
    endpoint: null,
    rx_bytes: 0,
    tx_bytes: 0,
    online: true,
    display_name: membership.name,
    gateway: null,
    path_mtu: null,
  };
}

/** Stable order by name: digits compare as numbers, so `pi-2` sorts before `pi-10`. */
export function byName(a: PeerInfo, b: PeerInfo): number {
  return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
}

/** This device first, then the others by name (§5 DeviceRow, §6.3). */
export function deviceItems(
  netId: string,
  peers: Readonly<Record<string, PeerInfo>> | undefined,
  self: PeerInfo | null,
): DeviceItem[] {
  const others = Object.values(peers ?? {})
    .slice()
    .sort(byName)
    .map((peer) => ({ peer, key: `${netId}/${peer.node}`, self: false }));
  return self ? [{ peer: self, key: `${netId}/${self.node}`, self: true }, ...others] : others;
}

/** Finds the device the URL names: by name, then by id or a short id prefix. */
export function resolveDevice(items: readonly DeviceItem[], param: string): DeviceItem | null {
  return (
    items.find((i) => i.peer.name === param) ??
    items.find(
      (i) => i.peer.node === param || (param.length >= 6 && i.peer.node.startsWith(param)),
    ) ??
    null
  );
}

export function isRelayed(path: PathKind | null): boolean {
  return path === 'relay' || path === 'stealth';
}

/** A plain WireGuard device, such as a phone, that connects through a gateway. */
export function isPhone(peer: PeerInfo): boolean {
  return peer.gateway !== null;
}

export type DeviceCounts = {
  readonly all: number;
  readonly online: number;
  readonly offline: number;
  readonly relayed: number;
  readonly phones: number;
  readonly exits: number;
};

/** The chip counts of §6.3: `All 12 · Online 11 · Offline 1 · Via relay 2 · Phones 2 · Exits 1`. */
export function countDevices(
  items: readonly DeviceItem[],
  exits: ReadonlySet<string>,
): DeviceCounts {
  let online = 0;
  let relayed = 0;
  let phones = 0;
  let exitCount = 0;
  for (const { peer } of items) {
    if (peer.online) online += 1;
    if (peer.online && isRelayed(peer.path)) relayed += 1;
    if (isPhone(peer)) phones += 1;
    if (exits.has(peer.node)) exitCount += 1;
  }
  return {
    all: items.length,
    online,
    offline: items.length - online,
    relayed,
    phones,
    exits: exitCount,
  };
}

export function matchesFilter(
  item: DeviceItem,
  filter: DevicesFilter | null,
  exits: ReadonlySet<string>,
): boolean {
  const { peer } = item;
  switch (filter) {
    case null:
      return true;
    case 'online':
      return peer.online;
    case 'offline':
      return !peer.online;
    case 'relayed':
      return peer.online && isRelayed(peer.path);
    case 'phones':
      return isPhone(peer);
    case 'exits':
      return exits.has(peer.node);
  }
}

/** `Filter by name or address`: a name or display name containing it, or an address. */
export function matchesQuery(item: DeviceItem, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q === '') return true;
  const { peer } = item;
  return (
    peer.name.toLowerCase().includes(q) ||
    peer.display_name.toLowerCase().includes(q) ||
    peer.address.includes(q)
  );
}

const FILTERS: readonly DevicesFilter[] = ['online', 'offline', 'relayed', 'phones', 'exits'];

/** The `?filter=` value, or null for All. */
export function parseFilter(value: string | null): DevicesFilter | null {
  return FILTERS.find((f) => f === value) ?? null;
}

/** A relay's name from its device id, or a short id when this device does not know it. */
export function relayName(
  peers: Readonly<Record<string, PeerInfo>> | undefined,
  relay: string | null,
): string | null {
  if (relay === null) return null;
  return peers?.[relay]?.name ?? relay.slice(0, 8);
}

/** `direct`, `via vps`, `stealth via vps` (§5 PathBadge). */
export function pathText(path: PathKind | null, relay: string | null): string {
  if (path === 'direct') return copy.path.direct;
  if (path === 'relay') return relay ? copy.path.via(relay) : copy.deviceView.relayed;
  if (path === 'stealth') return relay ? copy.path.stealthVia(relay) : copy.deviceView.stealth;
  return '';
}

/**
 * A daemon phrase as a sentence, with a full stop at the end. The first letter stays as
 * given: a phrase may start with a device name, which is always lowercase.
 */
export function sentence(text: string): string {
  const trimmed = text.trim();
  if (trimmed === '') return '';
  return /[.?]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}

/** `1.20 megabytes per second`, or `idle`, for accessible names. */
export function spokenRate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec < IDLE_BYTES_PER_SEC) return copy.units.idle;
  const units = copy.deviceView.spokenBytesPerSec;
  let n = bytesPerSec;
  let unit = 0;
  while (n >= 1000 && unit < units.length - 1) {
    n /= 1000;
    unit += 1;
  }
  const digits = n >= 100 ? 0 : n >= 10 ? 1 : 2;
  return `${Number(n.toFixed(digits))} ${units[unit] ?? ''}`;
}

/** `3 h ago` from a duration in ms. */
export function agoMs(ms: number): string {
  return ago(Math.max(0, ms) / 1000);
}

/** A connection from `node` that policy refused, if any (§12.4): the device being blocked. */
export function blockedTo(
  blocked: readonly BlockedGroup[],
  netId: string,
  node: string,
): BlockedGroup | null {
  return blocked.find((b) => b.network === netId && b.from === node && b.port !== null) ?? null;
}

/** A blocked group involving `node` either way: the row needs a person (§5 DeviceRow). */
export function blockedInvolving(
  blocked: readonly BlockedGroup[],
  netId: string,
  node: string,
): BlockedGroup | null {
  return blocked.find((b) => b.network === netId && (b.from === node || b.to === node)) ?? null;
}

export type VerdictAction = 'remove' | 'allow';

export type Verdict = {
  /** The sentence. */
  readonly text: string;
  /** The `path_reason` sentence behind `Learn why`, for a relayed path. */
  readonly reason: string | null;
  /** The one action that answers it, when one exists. */
  readonly action: VerdictAction | null;
  /** Whether the sentence describes a fault the person can act on. */
  readonly attention: boolean;
};

export type VerdictInput = {
  readonly peer: PeerInfo;
  /** The relay's name, for a relayed path. */
  readonly relay: string | null;
  /** The device it connects through (a phone's gateway), `this device` when it is this one. */
  readonly gateway?: string | null;
  readonly rttMs: number | null;
  /** How long ago the device was last heard from, in ms; null if never. */
  readonly seenAgoMs: number | null;
  /** A connection from this device to it that policy refused. */
  readonly blocked: BlockedGroup | null;
};

/** The one-sentence answer of the device detail (§12.4), first match wins. */
export function verdict(input: VerdictInput): Verdict {
  const { peer, relay, rttMs, seenAgoMs, blocked } = input;
  const v = copy.verdict;
  const more = copy.verdictMore;
  const plain = (text: string): Verdict => ({ text, reason: null, action: null, attention: false });
  if (seenAgoMs === null) return plain(isPhone(peer) ? more.waitingPhone : more.never);
  if (!peer.online) {
    if (seenAgoMs < RECENTLY_SEEN_MS) return plain(v.offlineRecent(agoMs(seenAgoMs)));
    if (seenAgoMs > LONG_GONE_MS) {
      return {
        // LONG_GONE_MS is one day, so this counts whole days.
        text: v.offlineLong(Math.floor(seenAgoMs / LONG_GONE_MS)),
        reason: null,
        action: 'remove',
        attention: false,
      };
    }
    return plain(more.offlineSome(agoMs(seenAgoMs)));
  }
  if (blocked && blocked.port !== null) {
    return {
      text: v.blocked(blocked.to_name, blocked.port),
      reason: null,
      action: 'allow',
      attention: true,
    };
  }
  if (peer.path_mtu !== null) return plain(v.smallMtu(peer.path_mtu));
  if (seenAgoMs > QUIET_HANDSHAKE_MS) return plain(v.quiet);
  const rtt = rttMs === null ? null : formatRtt(rttMs);
  if (input.gateway) {
    return plain(rtt ? v.relayed(input.gateway, rtt) : more.relayed(input.gateway));
  }
  if (isRelayed(peer.path)) {
    const name = relay ?? '';
    return {
      text: rtt ? v.relayed(name, rtt) : more.relayed(name),
      reason: peer.path_reason ? sentence(peer.path_reason) : null,
      action: null,
      attention: false,
    };
  }
  return plain(rtt ? v.direct(rtt) : more.direct);
}

/** Whether an entry is about `node` in `netId`. */
export function entryIsAbout(entry: Entry, netId: string, node: string, exitName: string): boolean {
  switch (entry.kind) {
    case 'peer_online':
    case 'peer_offline':
    case 'path_changed':
    case 'endpoint_changed':
    case 'device_joined':
    case 'device_removed':
    case 'device_renamed':
    case 'join_requested':
      return entry.network === netId && entry.node === node;
    case 'blocked':
      return entry.network === netId && (entry.from === node || entry.to === node);
    case 'exit_changed':
      return entry.exit !== null && entry.exit === exitName;
    case 'dropped':
    case 'warning':
    case 'log':
      return false;
  }
}

/** The last `limit` entries about a device, newest first (§6.4 History). */
export function historyOf(
  journal: readonly Entry[],
  netId: string,
  node: string,
  exitName: string,
  limit: number,
): Entry[] {
  const out: Entry[] = [];
  for (let i = journal.length - 1; i >= 0 && out.length < limit; i -= 1) {
    const entry = journal[i];
    if (entry && entryIsAbout(entry, netId, node, exitName)) out.push(entry);
  }
  return out;
}

/** Every entry about a device, oldest first, for the chart's event pins. */
export function eventsOf(
  journal: readonly Entry[],
  netId: string,
  node: string,
  exitName: string,
): Entry[] {
  return journal.filter((entry) => entryIsAbout(entry, netId, node, exitName));
}

/** When the device was last heard from, in server ms: its last offline entry, or the snapshot. */
export function lastSeenAt(
  journal: readonly Entry[],
  netId: string,
  peer: PeerInfo,
  snapshotSeenAt: number | null,
): number | null {
  for (let i = journal.length - 1; i >= 0; i -= 1) {
    const entry = journal[i];
    if (
      entry &&
      (entry.kind === 'peer_offline' || entry.kind === 'peer_online') &&
      entry.network === netId &&
      entry.node === peer.node
    ) {
      return snapshotSeenAt === null ? entry.at_ms : Math.max(entry.at_ms, snapshotSeenAt);
    }
  }
  return snapshotSeenAt;
}

/** When the device joined, from its `device_joined` entry, if the page still holds it. */
export function joinedAt(journal: readonly Entry[], netId: string, node: string): number | null {
  for (let i = journal.length - 1; i >= 0; i -= 1) {
    const entry = journal[i];
    if (entry?.kind === 'device_joined' && entry.network === netId && entry.node === node) {
      return entry.at_ms;
    }
  }
  return null;
}

/** How a device is present, for its dot and the words beside it (§5 DeviceRow states). */
export type Presence =
  | { readonly kind: 'online' }
  | { readonly kind: 'offline'; readonly seenAt: number }
  | { readonly kind: 'never'; readonly addedAt: number | null }
  | { readonly kind: 'waiting'; readonly addedAt: number | null };

export function presenceOf(
  peer: PeerInfo,
  seenAt: number | null,
  addedAt: number | null,
): Presence {
  if (peer.online) return { kind: 'online' };
  if (seenAt === null) return isPhone(peer) ? { kind: 'waiting', addedAt } : { kind: 'never', addedAt };
  return { kind: 'offline', seenAt };
}

/** Path changes in the last minute: more than three is `flapping` (§5 PathBadge). */
export function isFlapping(
  journal: readonly Entry[],
  netId: string,
  node: string,
  nowMs: number,
): boolean {
  let changes = 0;
  for (let i = journal.length - 1; i >= 0; i -= 1) {
    const entry = journal[i];
    if (!entry || entry.at_ms < nowMs - FLAP_WINDOW_MS) break;
    if (entry.kind === 'path_changed' && entry.network === netId && entry.node === node) changes += 1;
  }
  return changes > FLAP_COUNT;
}

/** The full name a device answers to, such as `nas.home.kith.internal`. */
export function fqdn(name: string, network: string): string {
  return copy.detail.fqdn(name, network);
}

/** Names an admin set that point at the device, by address or by its name (§6.4). */
export function namesPointingAt(
  records: readonly RecordView[],
  peer: PeerInfo,
  network: string,
): RecordView[] {
  const own = fqdn(peer.name, network);
  return records.filter((record) => {
    if (record.device) return false;
    const value = record.value.replace(/\.$/, '');
    return value === peer.address || value === own;
  });
}

/** Bytes received and sent in the window ending at `nowMs`; counter resets are dropped (§9.1). */
export function windowTotals(
  columns: TrafficColumns | undefined,
  windowMs: number,
  nowMs: number,
): { rx: number; tx: number } {
  let rx = 0;
  let tx = 0;
  if (!columns) return { rx, tx };
  const from = nowMs - windowMs;
  for (let i = 1; i < columns.length; i += 1) {
    if ((columns.at[i] ?? 0) <= from) continue;
    const drx = (columns.rx[i] ?? 0) - (columns.rx[i - 1] ?? 0);
    const dtx = (columns.tx[i] ?? 0) - (columns.tx[i - 1] ?? 0);
    if (drx > 0) rx += drx;
    if (dtx > 0) tx += dtx;
  }
  return { rx, tx };
}

/** `100.64.0.7 · 34 ms · ↓ 1.2 MB/s` and the like: facts joined by the dot. */
export function facts(parts: readonly (string | null | false)[]): string {
  return parts.filter((p): p is string => typeof p === 'string' && p !== '').join(DOT);
}
