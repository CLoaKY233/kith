// What the signed-in person may do with people and the network (docs/plans/console.md).
// Owners and admins run the network; only the owner makes or unmakes admins, because
// SetRole is owner-only in the ledger. Members see everything read-only. The server
// checks all of it again; these only decide what the page shows.

import type { Me, Person, PersonRole } from './types';

export function runsNetwork(role: PersonRole | undefined): boolean {
  return role === 'owner' || role === 'admin';
}

/** Roles `me` may give a new person, in the order the form offers them. */
export function addableRoles(me: Me | null): readonly PersonRole[] {
  if (me?.role === 'owner') return ['member', 'admin'];
  if (me?.role === 'admin') return ['member'];
  return [];
}

/** Roles `me` may move `person` to; empty when nothing can change. */
export function roleChoices(me: Me | null, person: Person): readonly PersonRole[] {
  if (me?.role !== 'owner' || person.id === me.id || person.role === 'owner') return [];
  return person.role === 'admin' ? ['member'] : ['admin'];
}

export function canRemove(me: Me | null, person: Person): boolean {
  if (!me || person.id === me.id || person.role === 'owner') return false;
  if (me.role === 'owner') return true;
  return me.role === 'admin' && person.role === 'member';
}

/** People in the order the list shows them: the owner, admins, members; by name within. */
export function sortPeople(people: readonly Person[]): Person[] {
  const rank: Record<PersonRole, number> = { owner: 0, admin: 1, member: 2 };
  return [...people].sort((a, b) => rank[a.role] - rank[b.role] || a.name.localeCompare(b.name));
}
