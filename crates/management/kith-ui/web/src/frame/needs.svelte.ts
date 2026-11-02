// What needs this person (spec §5 NeedsYouCard, §6.2): join requests (admins only),
// blocked connections (admins allow, members ask), and health from `problems` and
// `warnings`. One list feeds the Overview strip, the header popover, the mobile tab
// badge and `?need=`.
//
// A card the person acted on is held after it leaves `live.pending`, so its outcome
// (`Signed. Spreading`, `Approved by laptop`) stays readable until the card releases it.

import { untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import { announce } from '../lib/announce';
import { copy } from '../lib/copy';
import { problemId } from '../lib/kith/headline';
import { live } from '../lib/live/store.svelte';
import { BLOCKED_ANNOUNCE_MS } from '../lib/live/timing';
import type { Need } from '../lib/types';
import type { BlockedGroup, JoinRequest } from '../lib/types.gen';
import { isAdmin, networkName } from './net';
import { prefs } from './prefs.svelte';

export type JoinNeed = Need & { readonly kind: 'join'; readonly join: JoinRequest };
export type BlockedNeed = Need & { readonly kind: 'blocked'; readonly blocked: BlockedGroup };
export type HealthNeed = Need & {
  readonly kind: 'health';
  readonly what: string;
  /** How to fix it, for a network problem. */
  readonly fix: string | null;
  /** When a warning was logged, in server ms; null for a problem (it is ongoing). */
  readonly atMs: number | null;
};
export type NeedDetail = JoinNeed | BlockedNeed | HealthNeed;

/** Daemon-wide warnings belong to no network; they show in every one. */
const ANY_NETWORK = '';

class Needs {
  readonly #held = new SvelteMap<string, NeedDetail>();

  /** Everything waiting for this person now, joins first, then blocked, then health. */
  readonly live: readonly NeedDetail[] = $derived.by(() => {
    const now = live.now();
    const out: NeedDetail[] = [];
    for (const join of live.pending.joins) {
      if (!isAdmin(join.network) || prefs.ignoredAt(join.id, now)) continue;
      out.push({ id: join.id, kind: 'join', network: join.network, join });
    }
    for (const blocked of live.pending.blocked) {
      if (prefs.ignoredAt(blocked.id, now)) continue;
      out.push({ id: blocked.id, kind: 'blocked', network: blocked.network, blocked });
    }
    for (const net of live.status?.networks ?? []) {
      for (const problem of net.problems) {
        out.push({
          id: problemId(net.id, problem.what),
          kind: 'health',
          network: net.id,
          what: problem.what,
          fix: problem.fix,
          atMs: null,
        });
      }
    }
    const warnings = live.status?.warnings ?? [];
    for (const warning of [...warnings].reverse()) {
      const id = `warning:${warning.message}`;
      if (prefs.ignoredAt(id, now)) continue;
      out.push({
        id,
        kind: 'health',
        network: ANY_NETWORK,
        what: warning.message,
        fix: null,
        atMs: now - warning.secs_ago * 1000,
      });
    }
    return out;
  });

  /** Needs for one network, with held cards kept where the person left them. */
  forNet(net: string): readonly NeedDetail[] {
    const own = this.live.filter((n) => n.network === net || n.network === ANY_NETWORK);
    const ids = new Set(own.map((n) => n.id));
    const held = [...this.#held.values()].filter((n) => n.network === net && !ids.has(n.id));
    return [...own, ...held];
  }

  /**
   * How many live needs this person has in a network. Held outcomes do not count, and
   * neither does an expired join request: its card stays to say so, but nothing waits.
   */
  count(net: string): number {
    const now = live.now();
    return this.live.filter(
      (n) =>
        (n.network === net || n.network === ANY_NETWORK) &&
        !(n.kind === 'join' && now >= n.join.expires_at_ms),
    ).length;
  }

  /**
   * Join requests in a network still asking: not expired, and not one the person is
   * approving or denying here (held while Undo runs or the outcome shows).
   */
  asking(net: string): readonly JoinNeed[] {
    const now = live.now();
    return this.live.filter(
      (n): n is JoinNeed =>
        n.kind === 'join' && n.network === net && now < n.join.expires_at_ms && !this.#held.has(n.id),
    );
  }

  /** One need by id, live or held; null once it is gone. */
  find(id: string): NeedDetail | null {
    return this.live.find((n) => n.id === id) ?? this.#held.get(id) ?? null;
  }

  /** Keeps a card on screen after it leaves `live.pending`. */
  hold(need: NeedDetail): void {
    this.#held.set(need.id, need);
  }

  release(id: string): void {
    this.#held.delete(id);
  }

  /**
   * Announces new join requests and blocked groups politely (§11.1): requests present at
   * boot are not news, and one blocked triple is spoken at most once per 5 minutes.
   * Call once from App.svelte's setup.
   */
  track(): void {
    const seen: Record<string, true> = {};
    const spoken: Record<string, number> = {};
    let primed = false;
    $effect(() => {
      const list = this.live;
      if (!live.ready) return;
      untrack(() => {
        const now = live.now();
        for (const need of list) {
          if (need.kind === 'join' && !seen[need.id]) {
            seen[need.id] = true;
            if (primed) announce(copy.overview.joinOne(need.join.claimed.name, networkName(need.network)));
          } else if (need.kind === 'blocked') {
            const b = need.blocked;
            const triple = `${b.from}/${b.to}/${b.port ?? b.protocol}`;
            const last = spoken[triple];
            if (last !== undefined && now - last < BLOCKED_ANNOUNCE_MS) continue;
            spoken[triple] = now;
            if (primed) {
              const port = b.port === null ? '' : copy.blocked.port(b.port, copy.services[b.port] ?? null);
              announce(copy.blocked.card(b.from_name, b.to_name, port, b.count));
            }
          }
        }
        primed = true;
      });
    });
  }
}

export const needs = new Needs();
