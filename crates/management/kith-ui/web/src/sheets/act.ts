// Shared logic for the action flows of spec §7.5, §7.6 and §7.9 to §7.11, and the friction
// tiers of §13: which network a flow is about, how names are typed and checked, and how an
// action's progress and failures read. All pure and unit-tested except normalizeInput,
// which edits a field in place.

import { copy } from '../lib/copy';
import type { ActionHandle, ApiFailure, ConnState } from '../lib/types';
import type {
  Membership,
  NameCheck,
  NetworkSummary,
  PeerInfo,
  RecordView,
  StatusView,
} from '../lib/types.gen';

/** The network a flow is about, found by id or by name (routes use names, the API ids). */
export type NetworkContext = {
  readonly id: string;
  readonly name: string;
  readonly admin: boolean;
  readonly membership: Membership | null;
};

export function networkContext(status: StatusView | null, net: string): NetworkContext | null {
  const summary: NetworkSummary | undefined =
    status?.networks.find((n) => n.id === net) ?? status?.networks.find((n) => n.name === net);
  if (!summary) return null;
  return {
    id: summary.id,
    name: summary.name,
    admin: summary.admin,
    membership: status?.memberships.find((m) => m.network === summary.id) ?? null,
  };
}

/** §8.5: while stale or with the daemon down, nothing can change and nothing is queued. */
export function cannotAct(conn: ConnState): boolean {
  return conn === 'stale' || conn === 'daemon_down';
}

/** §7.1, §7.11: a capital lowercases, a space becomes a hyphen. */
export function normalizeName(raw: string): { value: string; spaced: boolean } {
  return { value: raw.toLowerCase().replace(/\s/g, '-'), spaced: /\s/.test(raw) };
}

/** Normalizes a name field as it is typed, keeping the caret where it was. */
export function normalizeInput(input: HTMLInputElement): { value: string; spaced: boolean } {
  const next = normalizeName(input.value);
  if (next.value !== input.value) {
    const { selectionStart, selectionEnd } = input;
    input.value = next.value;
    input.setSelectionRange(selectionStart, selectionEnd);
  }
  return next;
}

/** §7.6: `phone`, or `phone-2` (then `-3`, …) when a device in the network has it. */
export function freeName(base: string, names: readonly string[]): string {
  const taken = new Set(names);
  if (!taken.has(base)) return base;
  let n = 2;
  while (taken.has(`${base}-${n}`)) n += 1;
  return `${base}-${n}`;
}

/** §12.5 InvalidArgument, field-specific: the sentence for a name the validator refused. */
export function nameProblemText(check: NameCheck, network: string): string | null {
  switch (check.problem) {
    case null:
      return null;
    case 'empty':
      return copy.error.nameEmpty;
    case 'too_long':
      return copy.error.nameTooLong;
    case 'invalid_character':
      return copy.error.nameChars;
    case 'hyphen_at_edge':
      return copy.error.nameHyphenEdge;
    case 'taken':
      return copy.rename.taken(check.name, network, check.suggestion ?? check.name);
  }
}

/** §12.5: what a failed action says, cause then next step. `subject` names the device. */
export function failureText(error: ApiFailure, subject: string): string {
  switch (error.code) {
    case 'permission_denied':
    case 'forbidden':
      return copy.error.permissionDenied;
    case 'not_found':
      return copy.error.notFound(subject);
    case 'conflict':
      return copy.error.conflict(error.conflict?.resolved_by ?? subject);
    case 'invalid_argument':
      return copy.error.nameChars;
    case 'os_auth_denied':
      return copy.error.osAuthDenied;
    case 'daemon_down':
    case 'not_in_group':
    case 'version_skew':
      return copy.conn.staleActions;
    case 'unauthorized':
    case 'session_used':
    case 'session_expired':
      return copy.conn.unauth;
    case 'internal':
      return copy.error.internal(error.message);
  }
}

/**
 * §8.6: where a ledger op stands, as one line. `spreading` is the flow's own wording of
 * `Spreading · n of m`. Without acks ([fallback]) it never shows a count.
 */
export function progressText(
  handle: ActionHandle<unknown>,
  spreading: (n: number, m: number) => string = copy.action.spreading,
): string | null {
  switch (handle.state) {
    case 'pending':
      return copy.action.signing;
    case 'awaiting_os_auth':
      return copy.tier3.waiting;
    case 'accepted':
    case 'seen':
      if (handle.seen) return spreading(handle.seen.by, handle.seen.of);
      return handle.slow ? copy.action.savedWaiting : copy.action.savedHere;
    case 'idle':
    case 'conflict':
    case 'failed':
      return null;
  }
}

/** True once every device that was online has heard about the op. */
export function fullySeen(handle: ActionHandle<unknown>): boolean {
  return handle.seen !== null && handle.seen.by >= handle.seen.of;
}

/** §7.11: names set by an admin that point at a device, which follow it when renamed. */
export function namesPointingAt(
  records: readonly RecordView[],
  device: Pick<PeerInfo, 'name' | 'address'>,
  network: string,
): number {
  const fqdn = copy.detail.fqdn(device.name, network);
  const targets = new Set([
    device.address,
    device.name,
    `${device.name}${copy.names.suffix(network)}`,
    fqdn,
    `${fqdn}.`,
  ]);
  return records.filter((r) => !r.device && r.type !== 'TXT' && targets.has(r.value)).length;
}
