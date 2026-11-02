// Helpers the flow sheets share: which network a route names, how a failed action reads,
// how a typed name is normalised and judged, and how a path reads in a sentence.

import { copy } from '../lib/copy';
import { live } from '../lib/live/store.svelte';
import type { ApiFailure } from '../lib/types';
import type { Membership, NameProblem, NetworkSummary, PathKind } from '../lib/types.gen';

/** The network a route names: by name first, then by id (§4.1). */
export function findNetwork(net: string): NetworkSummary | null {
  const networks = live.status?.networks ?? [];
  return networks.find((n) => n.name === net) ?? networks.find((n) => n.id === net) ?? null;
}

/** This device in a network, by network id. */
export function membershipOf(networkId: string): Membership | null {
  return live.status?.memberships.find((m) => m.network === networkId) ?? null;
}

/** True while changes must wait for kith to come back (§8.5 stale, daemon_down). */
export function isStale(): boolean {
  return live.conn === 'stale' || live.conn === 'daemon_down';
}

/** A failed action as one sentence: cause, then the next step (§12.5). */
export function failureText(failure: ApiFailure): string {
  switch (failure.code) {
    case 'permission_denied':
    case 'forbidden':
      return copy.error.permissionDenied;
    case 'os_auth_denied':
      return copy.error.osAuthDenied;
    case 'daemon_down':
      return copy.conn.staleActions;
    case 'unauthorized':
    case 'session_used':
    case 'session_expired':
      return copy.conn.unauth;
    default:
      return copy.error.internal(failure.message);
  }
}

/**
 * A name as the person types it, normalised the way the CLI stores it: capitals lowered and
 * spaces made hyphens (§7.1). `spaced` says whether a space was replaced.
 */
export function normaliseName(raw: string): { value: string; spaced: boolean } {
  const value = raw.toLowerCase().replace(/ /g, '-');
  return { value, spaced: raw.includes(' ') };
}

/** The field error for a name problem the validator reported. */
export function nameProblemText(problem: NameProblem, taken: string): string {
  switch (problem) {
    case 'empty':
      return copy.error.nameEmpty;
    case 'too_long':
      return copy.error.nameTooLong;
    case 'invalid_character':
      return copy.error.nameChars;
    case 'hyphen_at_edge':
      return copy.error.nameHyphenEdge;
    case 'taken':
      return taken;
  }
}

/** A device's name in a network, from its id. */
export function deviceName(networkId: string, node: string | null): string | null {
  if (node === null) return null;
  return live.peers[networkId]?.[node]?.name ?? null;
}

/** `direct`, `via vps` or `stealth via vps`, for sentences such as the online line. */
export function pathText(path: PathKind | null, relay: string | null): string | null {
  switch (path) {
    case 'direct':
      return copy.path.direct;
    case 'relay':
      return copy.path.via(relay ?? '');
    case 'stealth':
      return copy.path.stealthVia(relay ?? '');
    case null:
      return null;
  }
}

/** What someone pasted to join, once recognised (§7.2). */
export type JoinTarget =
  | { readonly kind: 'invite'; readonly link: string }
  | {
      readonly kind: 'request';
      readonly code: string;
      readonly network: string | null;
      readonly admin: string | null;
    };

// Protocol forms, not copy: `kith://join/…`, `kith://request/…` and `network@admin`.
const INVITE = /^kith:\/\/join\/\S+$/i;
const REQUEST = /^kith:\/\/request\/\S+$/i;
const NETWORK_AT_ADMIN = /^([a-z0-9-]+)@([a-z0-9-]+)$/i;
/** A pasted `kith join …` command, whose argument is what counts. */
const COMMAND = /^kith\s+join\s+/i;

/** An invite link or a request target, from a link, code or pasted command; null if neither. */
export function recognise(raw: string): JoinTarget | null {
  const text = raw.trim().replace(COMMAND, '');
  if (INVITE.test(text)) return { kind: 'invite', link: text };
  if (REQUEST.test(text)) return { kind: 'request', code: text, network: null, admin: null };
  const at = NETWORK_AT_ADMIN.exec(text);
  if (!at) return null;
  const network = (at[1] ?? '').toLowerCase();
  const admin = (at[2] ?? '').toLowerCase();
  return { kind: 'request', code: `${network}@${admin}`, network, admin };
}
