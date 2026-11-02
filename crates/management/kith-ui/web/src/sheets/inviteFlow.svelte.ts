// The invite flow (spec §7.4) as state, so the Invite sheet and AddDevice's `Send an invite
// link` tab share one behaviour while each places the `Create invite` button in its own
// footer.

import { actions } from '../lib/live/store.svelte';
import { INVITE_VALIDITY_SECS, UNDO_MS } from '../lib/live/timing';
import type { ActionHandle, ApiFailure } from '../lib/types';
import type { Invite, Outcome } from '../lib/types.gen';

export type Validity = keyof typeof INVITE_VALIDITY_SECS;
export type UsesChoice = 'one' | 'five' | 'unlimited';

/** `Uses`: 1 · 5 · Unlimited, as the server takes them (`null` is unlimited). */
/** A sheet reopened this soon after making an invite shows it again, not a fresh form. */
export const INVITE_KEEP_MS = 60_000;

export const USES: Readonly<Record<UsesChoice, number | null>> = { one: 1, five: 5, unlimited: null };

export class InviteFlow {
  validity = $state<Validity>('hour');
  uses = $state<UsesChoice>('one');
  creating = $state<ActionHandle<Invite> | null>(null);
  revoking = $state<ActionHandle<Outcome> | null>(null);
  /** The invite shown has lapsed: the footer offers a new one. */
  expired = $state(false);
  /** When the invite was made, by the server's clock: joins after it may have used it. */
  createdAtMs = $state(0);

  /** The invite once the server made it. */
  readonly invite: Invite | null = $derived(
    this.creating?.state === 'accepted' || this.creating?.state === 'seen'
      ? this.creating.result
      : null,
  );
  readonly pending: boolean = $derived(this.creating?.state === 'pending');

  readonly #networkId: () => string | null;
  readonly #now: () => number;

  constructor(networkId: () => string | null, now: () => number) {
    this.#networkId = networkId;
    this.#now = now;
  }

  /** Signs nothing beyond the invite itself (tier 1, no extra confirmation). */
  async create(): Promise<void> {
    const net = this.#networkId();
    if (net === null || this.pending) return;
    this.revoking = null;
    this.createdAtMs = this.#now();
    const handle = actions.createInvite(net, INVITE_VALIDITY_SECS[this.validity], USES[this.uses]);
    this.creating = handle;
    await handle.done;
  }

  /**
   * Revokes at once (tier 0, with Undo): the form comes back straight away and the server
   * hears of it only once the Undo window closes. Returns the Undo, or null if there is
   * nothing to revoke. `onFailed` hears a revoke the server refused.
   */
  revoke(onFailed: (error: ApiFailure) => void): (() => void) | null {
    const net = this.#networkId();
    const invite = this.invite;
    const made = this.creating;
    if (net === null || invite === null || made === null) return null;
    const at = this.createdAtMs;
    this.reset();
    const timer = setTimeout(() => {
      const handle = actions.revokeInvite(net, invite.id);
      this.revoking = handle;
      void handle.done.then((settled) => {
        if (settled.state === 'failed' && settled.error) onFailed(settled.error);
      });
    }, UNDO_MS);
    return () => {
      clearTimeout(timer);
      if (this.creating !== null) return;
      this.creating = made;
      this.createdAtMs = at;
    };
  }

  /** Back to the empty form, keeping the chosen validity and uses. */
  reset(): void {
    this.expired = false;
    this.creating = null;
    this.revoking = null;
    this.createdAtMs = 0;
  }
}
