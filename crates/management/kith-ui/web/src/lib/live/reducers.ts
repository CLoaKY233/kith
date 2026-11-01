// Journal entries applied to the store, one reducer per EntryKind (spec §8.2). The switch
// is exhaustive: a new kind in types.gen.ts fails the type check until it is handled here.

import type { Entry, PeerInfo, Pending, StatusView } from '../types.gen';

/** The part of the store reducers write. */
export interface ReducerState {
  status: StatusView | null;
  peers: Record<string, Record<string, PeerInfo>>;
  pending: Pending;
  /** Elements to tint for TINT_MS: keys are `net/node`, `exit` or `records/net`. */
  changed: Set<string>;
}

function assertNever(value: never): never {
  throw new Error(`unhandled entry kind: ${JSON.stringify(value)}`);
}

function peer(state: ReducerState, network: string, node: string): PeerInfo | undefined {
  return state.peers[network]?.[node];
}

/** Applies one entry. Returns false when it changes nothing the store holds. */
export function reduce(state: ReducerState, entry: Entry): boolean {
  switch (entry.kind) {
    case 'peer_online': {
      const p = peer(state, entry.network, entry.node);
      if (!p) return false;
      p.online = true;
      p.path = entry.path;
      p.gateway = entry.gateway;
      p.last_seen_secs = 0;
      state.changed.add(`${entry.network}/${entry.node}`);
      return true;
    }
    case 'peer_offline': {
      const p = peer(state, entry.network, entry.node);
      if (!p) return false;
      p.online = false;
      state.changed.add(`${entry.network}/${entry.node}`);
      return true;
    }
    case 'path_changed': {
      const p = peer(state, entry.network, entry.node);
      if (!p) return false;
      p.path = entry.to;
      state.changed.add(`${entry.network}/${entry.node}`);
      return true;
    }
    case 'endpoint_changed': {
      const p = peer(state, entry.network, entry.node);
      if (!p) return false;
      p.endpoint = entry.endpoint;
      return true;
    }
    case 'device_joined': {
      const net = (state.peers[entry.network] ??= {});
      net[entry.node] ??= {
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
      state.pending.joins = state.pending.joins.filter(
        (join) => !(join.network === entry.network && join.node === entry.node),
      );
      state.changed.add(`${entry.network}/${entry.node}`);
      return true;
    }
    case 'device_removed': {
      const net = state.peers[entry.network];
      if (!net || !(entry.node in net)) return false;
      state.peers[entry.network] = Object.fromEntries(
        Object.entries(net).filter(([node]) => node !== entry.node),
      );
      return true;
    }
    case 'exit_changed': {
      if (!state.status) return false;
      state.status.exit_node = entry.exit;
      state.changed.add('exit');
      return true;
    }
    case 'join_requested':
    case 'blocked':
      // The full request or group (check code, counts) comes from /api/pending, which the
      // store refetches when it sees these.
      return true;
    case 'dropped':
    case 'warning':
    case 'log':
      // Activity only: the journal ring holds them.
      return true;
    default:
      return assertNever(entry);
  }
}
