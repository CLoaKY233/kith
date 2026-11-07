import { describe, expect, it } from 'vitest';
import { addableRoles, canRemove, roleChoices, sortPeople } from './people';
import { viewerStatus } from './viewer';
import type { Me, Person, PersonRole } from './types';
import type { StatusView } from './types.gen';

const person = (id: string, role: PersonRole): Person => ({ id, name: id, role, pending: false, devices: [] });
const as = (role: PersonRole): Me => ({ id: 'me', name: 'me', role });

const owner = person('ana', 'owner');
const admin = person('bo', 'admin');
const member = person('cy', 'member');

describe('who may do what with people', () => {
  it('lets the owner add admins, admins add members, members nothing', () => {
    expect(addableRoles(as('owner'))).toEqual(['member', 'admin']);
    expect(addableRoles(as('admin'))).toEqual(['member']);
    expect(addableRoles(as('member'))).toEqual([]);
    expect(addableRoles(null)).toEqual([]);
  });

  it('lets only the owner change roles, never their own or the owner', () => {
    expect(roleChoices(as('owner'), admin)).toEqual(['member']);
    expect(roleChoices(as('owner'), member)).toEqual(['admin']);
    expect(roleChoices(as('owner'), owner)).toEqual([]);
    expect(roleChoices({ ...as('owner'), id: 'bo' }, { ...admin, role: 'member' })).toEqual([]);
    expect(roleChoices(as('admin'), member)).toEqual([]);
  });

  it('lets admins remove members, the owner anyone but the owner, nobody themselves', () => {
    expect(canRemove(as('admin'), member)).toBe(true);
    expect(canRemove(as('admin'), admin)).toBe(false);
    expect(canRemove(as('owner'), admin)).toBe(true);
    expect(canRemove(as('owner'), owner)).toBe(false);
    expect(canRemove(as('member'), member)).toBe(false);
    expect(canRemove({ ...as('admin'), id: 'cy' }, member)).toBe(false);
  });

  it('lists the owner, then admins, then members, by name', () => {
    const list = [person('zed', 'member'), member, admin, owner, person('al', 'admin')];
    expect(sortPeople(list).map((p) => p.id)).toEqual(['ana', 'al', 'bo', 'cy', 'zed']);
  });
});

describe('viewerStatus', () => {
  const status = {
    networks: [{ id: 'n', name: 'home', admin: true }],
    memberships: [{ network: 'n', name: 'console', role: 'admin' }],
  } as unknown as StatusView;

  it('gives a signed-in member the member view of an admin device', () => {
    const seen = viewerStatus({ ...status, me: as('member') });
    expect(seen.networks[0]?.admin).toBe(false);
    expect(seen.memberships[0]?.role).toBe('member');
  });

  it('leaves admins, owners and local mode as the device is', () => {
    expect(viewerStatus({ ...status, me: as('admin') }).networks[0]?.admin).toBe(true);
    const local = { ...status, me: null };
    expect(viewerStatus(local)).toBe(local);
  });
});
