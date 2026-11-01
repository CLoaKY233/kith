// The action runner behind `actions.*` (spec §8.6). Every action becomes a reactive
// ActionHandle that moves through:
//
//   pending ──2xx──▶ accepted ──journal entry or ack──▶ seen
//      │                 └── SEEN_TIMEOUT_MS with no seen: `slow`
//      ├──409──▶ conflict (the other device's result, never an error)
//      └──other─▶ failed
//
// Tier-3 actions (§13) show `awaiting_os_auth` while the server waits for the OS to ask
// the person at the computer. Nothing is queued: while the connection is stale or the
// daemon is down, a change fails at once with `daemon_down` (§8.5).
//
// Without propagation acks (§2.12 [fallback]) `seen` stays null: a ledger op reaches
// `seen` when its own journal entry arrives, and the page says `Saved on this device`.

import { copy } from '../copy';
import { ApiError, errorText, type ErrorContext } from '../api';
import type { Entry } from '../types.gen';
import type { ActionHandle, Spread } from '../types';
import { Handle, type ActionSpec, type HandleOwner, type RunnerHost, type Tracked } from './handle.svelte';

export type { ActionSpec, RunnerHost };

// Ops still waiting for a seen, and acks waiting for their op, are kept this many at most.
const TRACKED_MAX = 32;

/** Runs actions and feeds them the journal and acks. One per store. */
export class Runner implements HandleOwner {
  readonly host: RunnerHost;
  private tracked = new Set<Tracked>();
  private acks = new Map<string, Spread>();
  private latest = new Map<string, AbortController>();

  constructor(host: RunnerHost) {
    this.host = host;
  }

  /** Starts an action. Changes fail at once while stale; reads always go out. */
  run<T>(spec: ActionSpec<T>): ActionHandle<T> {
    const conn = this.host.conn;
    const blocked =
      !spec.read && (conn === 'stale' || conn === 'daemon_down' || conn === 'ui_gone' || conn === 'unauth')
        ? new ApiError('daemon_down', 0, conn, null)
        : null;
    let controller: AbortController | null = null;
    if (spec.latest !== undefined) {
      this.latest.get(spec.latest)?.abort();
      controller = new AbortController();
      this.latest.set(spec.latest, controller);
    }
    const handle = new Handle<T>(spec, this, controller, blocked);
    if (spec.match && !blocked) this.track(handle);
    if (controller && spec.latest !== undefined) {
      const key = spec.latest;
      const mine = controller;
      void handle.done.then(() => {
        if (this.latest.get(key) === mine) this.latest.delete(key);
      });
    }
    return handle;
  }

  /** Feeds one journal entry to every action waiting for its seen. */
  observe(entry: Entry): void {
    for (const handle of [...this.tracked]) handle.observe(entry);
  }

  /**
   * Feeds a propagation ack (§2.12 `op_seen`): `by` of `of` devices have `op`. An ack that
   * arrives before the action's 2xx is kept until it does.
   */
  ack(op: string, by: number, of: number): void {
    const spread: Spread = { by, of };
    for (const handle of this.tracked) {
      if (handle.op === op) {
        handle.markSeen(spread);
        return;
      }
    }
    this.acks.set(op, spread);
    if (this.acks.size > TRACKED_MAX) {
      const oldest = this.acks.keys().next().value;
      if (oldest !== undefined) this.acks.delete(oldest);
    }
  }

  /** Internal: an ack that came before the op's own 2xx. */
  takeAck(op: string): Spread | null {
    const spread = this.acks.get(op) ?? null;
    this.acks.delete(op);
    return spread;
  }

  /** Internal: start feeding a handle. */
  track(handle: Tracked): void {
    this.tracked.add(handle);
    if (this.tracked.size > TRACKED_MAX) {
      const oldest = this.tracked.values().next().value;
      if (oldest !== undefined) this.tracked.delete(oldest);
    }
  }

  /** Internal: stop feeding a handle. */
  untrack(handle: Tracked): void {
    this.tracked.delete(handle);
  }
}

/**
 * The line a screen shows for a handle (§8.6, §13, §12.5), or null when the state needs
 * none: `Signing…`, `Waiting for confirmation on this computer…`, `Saved on this device`,
 * `Signed. Spreading · 2 of 3 devices`, the slow text, or the error sentence. A 409 reads
 * as who changed it; flows with their own race copy (§7.3 step 7) read `conflict` instead.
 */
export function progressText(handle: ActionHandle<unknown>, context: ErrorContext = {}): string | null {
  switch (handle.state) {
    case 'idle':
      return null;
    case 'pending':
      return handle.ledger ? copy.action.signing : null;
    case 'awaiting_os_auth':
      return copy.action.awaitingOsAuth;
    case 'accepted':
      if (!handle.ledger) return null;
      return handle.slow ? copy.action.savedWaiting : copy.action.savedHere;
    case 'seen':
      if (handle.seen) return copy.action.spreading(handle.seen.by, handle.seen.of);
      return handle.ledger ? copy.action.savedHere : null;
    case 'conflict':
      return handle.conflict ? copy.error.conflict(handle.conflict.resolved_by) : null;
    case 'failed':
      return handle.error ? errorText(handle.error, context) : null;
  }
}
