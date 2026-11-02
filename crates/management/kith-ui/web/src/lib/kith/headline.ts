// The Overview headline (spec §6.2): the first condition that matches wins. Pure, so the
// priority table is tested without a page.

import { copy } from '../copy';
import { ago, clock, rate, rateParts, type Parts } from '../format';
import type { ExitState, PeerInfo, Problem } from '../types.gen';

/**
 * Where a network problem's fix leads: the device it names, the command it gives (to copy),
 * or This device when neither.
 */
export type ProblemTarget =
  | { readonly kind: 'device'; readonly node: string }
  | { readonly kind: 'command'; readonly command: string }
  | { readonly kind: 'this' };

export type HeadlineAction =
  | { readonly kind: 'add_device'; readonly label: string }
  | { readonly kind: 'why'; readonly label: string; readonly node: string }
  | { readonly kind: 'fix'; readonly label: string; readonly target: ProblemTarget };

/** The default sub-line in parts, so the rate units can sit in `--ink-3` (§3.3). */
export type HeadlineSub = {
  readonly counts: string;
  readonly down: Parts;
  readonly up: Parts;
};

export type HeadlineView = {
  readonly text: string;
  /** `danger` only for a network problem (priority 2). */
  readonly tone: 'plain' | 'danger';
  readonly action: HeadlineAction | null;
  /** The default sub-line: `9 direct · 2 via vps · ↓ 3.4 MB/s ↑ 220 kB/s`. */
  readonly sub: string | null;
  readonly subParts: HeadlineSub | null;
};

export type HeadlineInput = {
  readonly network: string;
  readonly netId: string;
  readonly admin: boolean;
  readonly pausedUntilMs: number | null;
  readonly problems: readonly Problem[];
  readonly exit: { readonly name: string; readonly state: ExitState } | null;
  /** Join requests for this network, oldest first, by the name they ask for. */
  readonly joins: readonly string[];
  /** Other devices; this device is counted on top. */
  readonly peers: readonly PeerInfo[];
  /** Offline devices this device was using: exits, or traffic in the last hour. */
  readonly expected: readonly PeerInfo[];
  /** Relay name per online relayed device id. */
  readonly relayOf: Readonly<Record<string, string>>;
  readonly down: number;
  readonly up: number;
};

/** The id `?need=` uses for a network problem; frame/needs.svelte.ts builds the same. */
export function problemId(netId: string, what: string): string {
  return `problem:${netId}:${what}`;
}

/** A fix that is a command: quoted in backticks, or the whole fix is one. */
const QUOTED = /`([^`]+)`/;
const WHOLE = /^(?:sudo )?kith [^.]*$/;

/** What a problem's fix should do (see ProblemTarget). */
export function problemTarget(
  problem: Problem,
  peers: readonly Pick<PeerInfo, 'name' | 'node'>[],
): ProblemTarget {
  const words = new Set(problem.what.split(/[^\w.-]+/).map((w) => w.replace(/\.$/, '')));
  const named = peers.find((p) => words.has(p.name));
  if (named) return { kind: 'device', node: named.node };
  const text = QUOTED.exec(problem.fix)?.[1] ?? (WHOLE.test(problem.fix) ? problem.fix : null);
  if (text) return { kind: 'command', command: text };
  return { kind: 'this' };
}

function counts(input: HeadlineInput): string {
  const online = input.peers.filter((p) => p.online && p.path !== null);
  const direct = online.filter((p) => p.path === 'direct').length;
  const relayed = online.filter((p) => p.path !== 'direct');
  const counts = new Map<string, number>();
  for (const p of relayed) {
    const relay = input.relayOf[p.node];
    if (relay) counts.set(relay, (counts.get(relay) ?? 0) + 1);
  }
  const parts = [copy.overview.direct(direct)];
  for (const [relay, n] of [...counts].sort((a, b) => b[1] - a[1])) {
    parts.push(copy.overview.via(n, relay));
  }
  return parts.join(copy.overview.sep);
}

export function headline(input: HeadlineInput): HeadlineView {
  const plain = (text: string, action: HeadlineAction | null = null): HeadlineView => ({
    text,
    tone: 'plain',
    action,
    sub: null,
    subParts: null,
  });

  if (input.pausedUntilMs !== null) return plain(copy.overview.paused(clock(input.pausedUntilMs)));

  const problem = input.problems[0];
  if (problem) {
    return {
      text: problem.what,
      tone: 'danger',
      action: { kind: 'fix', label: problem.fix, target: problemTarget(problem, input.peers) },
      sub: null,
      subParts: null,
    };
  }

  if (input.exit && input.exit.state !== 'active') {
    return plain(
      input.exit.state === 'offline_blocked'
        ? copy.overview.exitOfflineBlocked(input.exit.name)
        : copy.overview.exitOfflineDirect(input.exit.name),
    );
  }

  const [first] = input.joins;
  if (input.admin && first !== undefined) {
    return plain(
      input.joins.length === 1
        ? copy.overview.joinOne(first, input.network)
        : copy.overview.joinMany(input.joins.length, input.network),
    );
  }

  const gone = input.expected[0];
  if (gone) {
    const seen = gone.last_seen_secs === null ? copy.units.none : ago(gone.last_seen_secs);
    return plain(copy.overview.unreachable(gone.name, seen), {
      kind: 'why',
      label: copy.overview.why,
      node: gone.node,
    });
  }

  if (input.peers.length === 0) {
    return plain(
      copy.overview.onlyThis,
      input.admin ? { kind: 'add_device', label: copy.overview.addDevice } : null,
    );
  }

  const total = input.peers.length + 1;
  const online = input.peers.filter((p) => p.online).length + 1;
  const line = counts(input);
  return {
    text: copy.overview.allGood(online, total),
    tone: 'plain',
    action: null,
    sub: [line, copy.overview.rates(rate(input.down), rate(input.up))].join(copy.overview.sep),
    subParts: { counts: line, down: rateParts(input.down), up: rateParts(input.up) },
  };
}
