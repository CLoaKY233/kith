// Journal entries applied to the store, one case per EntryKind (spec §8.2). The switch is
// exhaustive: a new kind in types.gen.ts fails the type check until it is handled here.
// Reducers only change the store's state; what follows from a change (a tint, a refetch,
// a path to settle) goes back to the store in an Effects record.

import type { Entry, PathKind, PeerInfo, Pending, StatusView } from '../types.gen';
import { deviceKey, tintKey } from './keys';

/** The part of the store reducers write. */
export interface ReducerState {
  status: StatusView | null;
  peers: Record<string, Record<string, PeerInfo>>;
  pending: Pending;
  /** When each device was last heard from, server ms; `null` if never. */
  seenAt: Record<string, number | null>;
}

/** What the store does after a batch of entries. */
export interface Effects {
  /** Elements to tint (§8.4). */
  tint: Set<string>;
  /**
   * Devices whose path may have changed: the path now, when the entry saying so was
   * made, and the times of changes from one kind to another (for `flapping`).
   */
  paths: Map<string, { path: PathKind | null; atMs: number; flaps: number[] }>;
  /** Devices that left; their traffic and numbers go too. */
  removed: Set<string>;
  /** Refetch /api/pending: a request or blocked group arrived or was settled. */
  pending: boolean;
  /** Refetch /api/status: fields only the server knows changed (exit state, warnings). */
  status: boolean;
  /** Refetch the whole snapshot: an entry names something this page does not hold. */
  resync: boolean;
}

export function noEffects(): Effects {
  return {
    tint: new Set(),
    paths: new Map(),
    removed: new Set(),
    pending: false,
    status: false,
    resync: false,
  };
}

/** `record` without `key`. */
export function omit<T>(record: Record<string, T>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));
}

function summary(state: ReducerState, network: string) {
  return state.status?.networks.find((net) => net.id === network);
}

/** Whether `exit_node` (`network/device`, by name or id) names this device. */
function isExit(state: ReducerState, network: string, peer: PeerInfo): boolean {
  const exit = state.status?.exit_node;
  if (!exit) return false;
  const cut = exit.lastIndexOf('/');
  const net = exit.slice(0, cut);
  const device = exit.slice(cut + 1);
  const netMatches = net === network || net === summary(state, network)?.name;
  return netMatches && (device === peer.node || device === peer.name);
}

function setOnline(state: ReducerState, network: string, peer: PeerInfo, online: boolean): boolean {
  if (peer.online === online) return false;
  peer.online = online;
  const net = summary(state, network);
  if (net) net.peers_online = Math.max(0, net.peers_online + (online ? 1 : -1));
  return true;
}

function notePath(fx: Effects, key: string, from: PathKind | null, to: PathKind | null, atMs: number): void {
  const flaps = fx.paths.get(key)?.flaps ?? [];
  if (from !== null && to !== null && from !== to) flaps.push(atMs);
  fx.paths.set(key, { path: to, atMs, flaps });
}

/** Applies one entry to `state`, recording what follows in `fx`. */
export function reduce(state: ReducerState, entry: Entry, fx: Effects): void {
  switch (entry.kind) {
    case 'peer_online': {
      const key = deviceKey(entry.network, entry.node);
      const peer = state.peers[entry.network]?.[entry.node];
      if (!peer) {
        fx.resync = true;
        return;
      }
      const before = peer.path;
      if (setOnline(state, entry.network, peer, true)) fx.tint.add(tintKey.device(entry.network, entry.node));
      peer.path = entry.path;
      peer.gateway = entry.gateway;
      peer.last_seen_secs = 0;
      state.seenAt[key] = entry.at_ms;
      notePath(fx, key, before, entry.path, entry.at_ms);
      if (isExit(state, entry.network, peer)) fx.status = true;
      return;
    }
    case 'peer_offline': {
      const key = deviceKey(entry.network, entry.node);
      const peer = state.peers[entry.network]?.[entry.node];
      if (!peer) return;
      const before = peer.path;
      if (setOnline(state, entry.network, peer, false)) fx.tint.add(tintKey.device(entry.network, entry.node));
      // A path exists only while online (PeerInfo.path). The last time it was heard from
      // stays as the ticks recorded it: going offline is decided after the silence.
      peer.path = null;
      notePath(fx, key, before, null, entry.at_ms);
      if (isExit(state, entry.network, peer)) fx.status = true;
      return;
    }
    case 'path_changed': {
      const peer = state.peers[entry.network]?.[entry.node];
      if (!peer) return;
      const before = peer.path;
      peer.path = entry.to;
      // The tint follows the badge, which waits for the path to settle (§5 PathBadge).
      notePath(fx, deviceKey(entry.network, entry.node), before, entry.to, entry.at_ms);
      return;
    }
    case 'endpoint_changed': {
      const peer = state.peers[entry.network]?.[entry.node];
      if (peer) peer.endpoint = entry.endpoint;
      return;
    }
    case 'device_joined': {
      if (entry.node === state.status?.node) {
        // This device joined a network: memberships, roles and peers all change.
        fx.resync = true;
        return;
      }
      const net = (state.peers[entry.network] ??= {});
      if (!(entry.node in net)) {
        net[entry.node] = {
          node: entry.node,
          name: entry.name,
          address: entry.address,
          path: null,
          path_reason: null,
          last_seen_secs: null,
          endpoint: null,
          rx_bytes: 0,
          tx_bytes: 0,
          online: false,
          display_name: entry.name,
          gateway: null,
          path_mtu: null,
        };
        state.seenAt[deviceKey(entry.network, entry.node)] = null;
        const summaryNet = summary(state, entry.network);
        if (summaryNet) summaryNet.peers_total += 1;
        fx.tint.add(tintKey.device(entry.network, entry.node));
      }
      const joins = state.pending.joins.filter(
        (join) => !(join.network === entry.network && join.node === entry.node),
      );
      if (joins.length !== state.pending.joins.length) state.pending.joins = joins;
      fx.pending = true;
      return;
    }
    case 'device_removed': {
      if (entry.node === state.status?.node) {
        // This device was removed: its role in the network is now `removed`.
        fx.resync = true;
        return;
      }
      const net = state.peers[entry.network];
      const peer = net?.[entry.node];
      if (!net || !peer) return;
      const summaryNet = summary(state, entry.network);
      if (summaryNet) {
        summaryNet.peers_total = Math.max(0, summaryNet.peers_total - 1);
        if (peer.online) summaryNet.peers_online = Math.max(0, summaryNet.peers_online - 1);
      }
      state.peers[entry.network] = omit(net, entry.node);
      const key = deviceKey(entry.network, entry.node);
      state.seenAt = omit(state.seenAt, key);
      fx.removed.add(key);
      fx.paths.delete(key);
      // Admins and exits may have named it.
      fx.status = true;
      fx.pending = true;
      return;
    }
    case 'device_renamed': {
      if (entry.node === state.status?.node) {
        const membership = state.status.memberships.find((m) => m.network === entry.network);
        if (membership) membership.name = entry.name;
      }
      const peer = state.peers[entry.network]?.[entry.node];
      if (peer) {
        if (peer.display_name === peer.name) peer.display_name = entry.name;
        peer.name = entry.name;
        fx.tint.add(tintKey.device(entry.network, entry.node));
      }
      // Exits and records name it; the server has the new names.
      fx.status = true;
      return;
    }
    case 'exit_changed': {
      if (!state.status) return;
      if (state.status.exit_node !== entry.exit) {
        state.status.exit_node = entry.exit;
        if (entry.exit === null) state.status.exit_state = null;
        fx.tint.add(tintKey.exit);
      }
      // Whether the new exit is up, and what happens if it is not, only the server knows.
      fx.status = true;
      return;
    }
    case 'join_requested':
    case 'blocked':
      // The full request or group (check code, counts, rule) comes from /api/pending.
      fx.pending = true;
      return;
    case 'warning':
      // Health cards read Status.warnings.
      fx.status = true;
      return;
    case 'dropped':
    case 'log':
      // Activity only: the journal ring holds them.
      return;
    default: {
      // A kind this page does not know yet stays in Activity and changes nothing else.
      const unknown: never = entry;
      void unknown;
      return;
    }
  }
}
