// Keys into the store's per-device maps and into `live.changed`. Screens build keys with
// these, never by hand, so the store and its readers agree.

/** `net/node`: the key of `live.numbers`, `live.traffic`, `live.seenAt`, `live.paths`. */
export const deviceKey = (network: string, node: string): string => `${network}/${node}`;

/** Elements that take the 4 s change tint (§8.4). */
export const tintKey = {
  /** A device's row, map node and detail header: online, offline, path kind, joined. */
  device: deviceKey,
  /** A device's latency cell, when its RTT more than doubled for RTT_TINT_TICKS ticks. */
  rtt: (network: string, node: string): string => `${deviceKey(network, node)}#rtt`,
  /** The same, from a device key. */
  rttOf: (device: string): string => `${device}#rtt`,
  /** The exit in use: the header ExitChip and the Exits radio list. */
  exit: 'exit',
  /** A Needs-you item that just arrived, by its id. */
  need: (id: string): string => `need/${id}`,
  /** A name in the Names table that was added, changed or removed, by its full name. */
  record: (network: string, name: string): string => `rec/${network}/${name}`,
} as const;
