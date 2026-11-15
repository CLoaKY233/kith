// What the signed-in person may do with people. Only an owner makes or unmakes admins;
// admins handle members only. The server checks all of it again.
import type { Me, PersonRole, PersonView } from '@kith/types';

const handles = (me: Me | null, role: PersonRole) => me?.role === 'owner' || (me?.role === 'admin' && role === 'member');
const isMe = (me: Me | null, person: PersonView) => person.me || person.id === me?.id;

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

export function canRemove(me: Me | null, person: PersonView): boolean {
  if (!me || isMe(me, person) || (person.state === 'active' && person.role === 'owner')) return false;
  return handles(me, person.role);
}

export function canApprove(me: Me | null, person: PersonView): boolean {
  return person.state === 'registered' && handles(me, person.role);
}
