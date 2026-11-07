// What the signed-in person may do with people and the network (docs/plans/console.md).
// Owners and admins run the network; only an owner makes or unmakes admins, because
// SetRole is owner-only in the ledger, and admins handle members only. Members see
// everything read-only. The server checks all of it again; these only decide what the
// page shows.

import type { Me, PersonRole, PersonView } from './types.gen';

export function runsNetwork(role: PersonRole | undefined): boolean {
  return role === 'owner' || role === 'admin';
}

/** Whether `me` may add, approve or remove someone who has or will get `role`. */
function handles(me: Me | null, role: PersonRole): boolean {
  if (me?.role === 'owner') return true;
  return me?.role === 'admin' && role === 'member';
}

function isMe(me: Me | null, person: PersonView): boolean {
  return person.me || person.id === me?.id;
}

/** Roles `me` may give a new person, in the order the form offers them. */
export function addableRoles(me: Me | null): readonly PersonRole[] {
  if (me?.role === 'owner') return ['member', 'admin'];
  if (me?.role === 'admin') return ['member'];
  return [];
}

/** Roles `me` may move `person` to; empty when nothing can change. */
export function roleChoices(me: Me | null, person: PersonView): readonly PersonRole[] {
  if (me?.role !== 'owner' || person.state !== 'active' || isMe(me, person) || person.role === 'owner') return [];
  return person.role === 'admin' ? ['member'] : ['admin'];
}

/** Whether `me` may remove `person`, or take back their invite. */
export function canRemove(me: Me | null, person: PersonView): boolean {
  if (!me || isMe(me, person) || (person.state === 'active' && person.role === 'owner')) return false;
  return handles(me, person.role);
}

/** Whether `me` may approve someone who made their passkey, which signs them in. */
export function canApprove(me: Me | null, person: PersonView): boolean {
  return person.state === 'registered' && handles(me, person.role);
}

/**
 * People in the order the list shows them: those waiting for approval first, then the
 * owner, admins, members, and invites not yet opened; by name within.
 */
export function sortPeople(people: readonly PersonView[]): PersonView[] {
  const state = { registered: 0, active: 1, invited: 2 } as const;
  const rank: Record<PersonRole, number> = { owner: 0, admin: 1, member: 2 };
  return [...people].sort(
    (a, b) => state[a.state] - state[b.state] || rank[a.role] - rank[b.role] || a.name.localeCompare(b.name),
  );
}
