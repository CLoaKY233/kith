// The invite flow (spec §7.4) as state, so the Invite sheet and AddDevice's `Send an invite
// link` tab share one behaviour while each places the `Create invite` button in its own
// footer.

import { actions } from '../lib/live/store.svelte';
import { INVITE_VALIDITY_SECS } from '../lib/live/timing';
import type { ActionHandle } from '../lib/types';
import type { Invite, Outcome } from '../lib/types.gen';

export type Validity = keyof typeof INVITE_VALIDITY_SECS;
export type UsesChoice = 'one' | 'five' | 'unlimited';

/** `Uses`: 1 · 5 · Unlimited, as the server takes them (`null` is unlimited). */
export const USES: Readonly<Record<UsesChoice, number | null>> = { one: 1, five: 5, unlimited: null };

export class InviteFlow {
  validity = $state<Validity>('hour');
  uses = $state<UsesChoice>('one');
  creating = $state<ActionHandle<Invite> | null>(null);
  revoking = $state<ActionHandle<Outcome> | null>(null);
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

  /** Revokes at once (tier 0). Resolves true when the invite is gone. */
  async revoke(): Promise<boolean> {
    const net = this.#networkId();
    const invite = this.invite;
    if (net === null || invite === null || this.revoking?.state === 'pending') return false;
    const handle = actions.revokeInvite(net, invite.id);
    this.revoking = handle;
    const settled = await handle.done;
    if (settled.state !== 'accepted' && settled.state !== 'seen') return false;
    this.reset();
    return true;
  }

  /** Back to the empty form, keeping the chosen validity and uses. */
  reset(): void {
    this.creating = null;
    this.revoking = null;
    this.createdAtMs = 0;
  }
}
