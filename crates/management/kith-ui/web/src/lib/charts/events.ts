// Journal entries on the detail chart (§9.1, correlation is mandatory): the path a device
// used over time, for relayed bands and the readout, and the sentence each event pin shows.

import { copy } from '../copy';
import type { Entry, PathKind } from '../types.gen';

/** One stretch of time on one path; `null` while offline or unknown. */
export type PathSpan = { readonly from: number; readonly to: number; readonly path: PathKind | null };

/** True for paths drawn as a relayed band: relayed and stealth both go through a relay. */
export function isRelayed(path: PathKind | null): boolean {
  return path === 'relay' || path === 'stealth';
}

/**
 * The paths `node` used over [fromMs, toMs], from its online, offline and path entries in
 * seq order. Without any entry, `current` holds for the whole window.
 */
export function pathSpans(
  events: readonly Entry[],
  node: string,
  current: PathKind | null,
  fromMs: number,
  toMs: number,
): PathSpan[] {
  const changes: { at: number; path: PathKind | null }[] = [];
  let before: PathKind | null | undefined;
  const ordered = [...events].sort((a, b) => a.seq - b.seq);
  for (const e of ordered) {
    if (e.kind === 'path_changed' && e.node === node) {
      before ??= e.from;
      changes.push({ at: e.at_ms, path: e.to });
    } else if (e.kind === 'peer_online' && e.node === node) {
      before ??= null;
      changes.push({ at: e.at_ms, path: e.path });
    } else if (e.kind === 'peer_offline' && e.node === node) {
      before ??= null;
      changes.push({ at: e.at_ms, path: null });
    }
  }
  if (changes.length === 0) return toMs > fromMs ? [{ from: fromMs, to: toMs, path: current }] : [];

  const spans: PathSpan[] = [];
  let path: PathKind | null = before ?? null;
  let start = fromMs;
  for (const change of changes) {
    if (change.at > fromMs) {
      const end = Math.min(change.at, toMs);
      if (end > start) spans.push({ from: start, to: end, path });
      start = end;
    }
    path = change.path;
    if (change.at >= toMs) return spans;
  }
  if (toMs > start) spans.push({ from: start, to: toMs, path });
  return spans;
}

/** The path at `t`, or null when `t` is outside every span. */
export function pathAt(spans: readonly PathSpan[], t: number): PathKind | null {
  for (const span of spans) if (t >= span.from && t < span.to) return span.path;
  const last = spans[spans.length - 1];
  return last && t >= last.to ? last.path : null;
}

/** `direct`, `via vps`, `stealth via vps`; `relayed` when the relay's name is unknown. */
export function pathWord(path: PathKind | null, relay: string | null): string {
  switch (path) {
    case 'direct':
      return copy.path.direct;
    case 'relay':
      return relay ? copy.path.via(relay) : copy.chart.relayed;
    case 'stealth':
      return relay ? copy.path.stealthVia(relay) : copy.chart.relayed;
    case null:
      return copy.units.none;
  }
}

/** Names a device id in sentences: its display name, else the id. */
export type NameOf = (node: string) => string;

/** The Activity sentence (§12.3) for one entry, as its pin's tooltip. */
export function sentence(e: Entry, nameOf: NameOf, relay: string | null): string {
  switch (e.kind) {
    case 'peer_online':
      return copy.entry.peerOnline(nameOf(e.node), pathWord(e.path, relay), null);
    case 'peer_offline':
      return copy.entry.peerOffline(nameOf(e.node));
    case 'path_changed': {
      const was = pathWord(e.from, relay);
      const device = nameOf(e.node);
      if (e.to === 'direct') return copy.entry.pathDirect(device, was);
      if (e.to === 'stealth') return copy.entry.pathStealth(device, relay ?? copy.units.none, was);
      if (e.to === 'relay') return copy.entry.pathRelayed(device, relay ?? copy.units.none, was);
      return copy.entry.peerOffline(device);
    }
    case 'endpoint_changed':
      return copy.entry.endpointChanged(nameOf(e.node));
    case 'device_joined':
      return copy.entry.deviceJoinedPlain(nameOf(e.node));
    case 'device_removed':
      return copy.entry.deviceRemovedPlain(nameOf(e.node));
    case 'device_renamed':
      return copy.entry.deviceRenamed(e.old_name, e.name);
    case 'exit_changed':
      return e.exit ? copy.entry.exitChanged(nameOf(e.exit.split('/').pop() ?? e.exit)) : copy.entry.exitCleared;
    case 'dropped':
      return copy.entry.dropped(e.reason, e.count);
    case 'warning':
      return e.message;
    case 'log':
      return e.message;
    case 'join_requested':
      return copy.entry.joinRequested(e.name, e.network);
    case 'blocked':
      return copy.entry.blocked(e.from_name, e.to_name, e.port === null ? '' : `:${e.port}`, e.count);
  }
}
