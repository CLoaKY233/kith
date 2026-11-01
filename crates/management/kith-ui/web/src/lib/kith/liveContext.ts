// The Activity templates' context, read from the live store: this device's names, network
// names and current relays. Reads are reactive inside markup and $derived.

import { live } from '../live/store.svelte';
import type { Context } from './activity';

export const liveContext: Context = {
  selfName(network) {
    const memberships = live.status?.memberships ?? [];
    const own =
      network === null ? memberships[0] : memberships.find((m) => m.network === network);
    return own?.name ?? live.status?.host.name ?? null;
  },
  networkName(network) {
    return live.status?.networks.find((n) => n.id === network)?.name ?? null;
  },
  relayName(network, node) {
    const self = live.status?.node ?? null;
    const own = live.links[network]?.find(
      (l) => l.to === node && (self === null || l.from === self),
    );
    if (!own?.relay) return null;
    return live.peers[network]?.[own.relay]?.name ?? null;
  },
};
