// Finds the network a route names. Routes address networks by name, falling back to a
// short id when two names collide (spec §4.1).

import type { Membership, NetworkSummary, PeerInfo, StatusView } from '../types.gen';

/** The network `net` names: by exact name first, then by id or id prefix. */
export function findNetwork(status: StatusView | null, net: string): NetworkSummary | null {
  const networks = status?.networks ?? [];
  return (
    networks.find((n) => n.name === net) ??
    networks.find((n) => n.id === net) ??
    networks.find((n) => n.id.startsWith(net)) ??
    null
  );
}

/** This device's standing in a network. */
export function membershipOf(status: StatusView | null, network: string): Membership | null {
  return status?.memberships.find((m) => m.network === network) ?? null;
}

/** A network's devices by name, this device excluded (the peers list never holds it). */
export function devicesByName(
  peers: Readonly<Record<string, Readonly<Record<string, PeerInfo>>>>,
  network: string,
): PeerInfo[] {
  return Object.values(peers[network] ?? {}).sort((a, b) => a.name.localeCompare(b.name));
}
