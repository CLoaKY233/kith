// Small reads over `live` for one network: names, roles, this device as a row. Plain
// functions, so they are reactive wherever they are called from markup or $derived.

import { live } from '../lib/live/store.svelte';
import type { Membership, NetworkSummary, PeerInfo } from '../lib/types.gen';

export function summary(net: string): NetworkSummary | null {
  return live.status?.networks.find((n) => n.id === net) ?? null;
}

export function membership(net: string): Membership | null {
  return live.status?.memberships.find((m) => m.network === net) ?? null;
}

/** The network's name, or its id when it is not known (yet). */
export function networkName(net: string): string {
  return summary(net)?.name ?? net;
}

export function isAdmin(net: string): boolean {
  return membership(net)?.role === 'admin';
}

/** Other devices in the network, as the store holds them. */
export function peers(net: string): PeerInfo[] {
  return Object.values(live.peers[net] ?? {});
}

/** A device's name, for ids the server sends without one (relays, exits). */
export function deviceName(net: string, node: string): string {
  if (node === live.status?.node) return membership(net)?.name ?? node;
  return live.peers[net]?.[node]?.name ?? node;
}

/**
 * This device as a row: the peers list never includes it (§6.2 pins it first). Its numbers
 * are not a link, so path and traffic stay empty.
 */
export function selfPeer(net: string): PeerInfo | null {
  const me = membership(net);
  const node = live.status?.node;
  if (!me || !node) return null;
  return {
    node,
    name: me.name,
    address: me.address,
    path: null,
    path_reason: null,
    last_seen_secs: 0,
    endpoint: null,
    rx_bytes: 0,
    tx_bytes: 0,
    online: true,
    display_name: me.name,
    gateway: null,
    path_mtu: null,
  };
}

/** The exit in use: `exit_node` is `network/device`. */
export function exitName(): string | null {
  const exit = live.status?.exit_node;
  if (!exit) return null;
  const slash = exit.lastIndexOf('/');
  return slash >= 0 ? exit.slice(slash + 1) : exit;
}

/** `⌘K` on Apple platforms, `Ctrl K` elsewhere. */
export const apple =
  typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
