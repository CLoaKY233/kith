import { describe, expect, it } from 'vitest';
import { addableRoles, canApprove, canRemove, roleChoices, sortPeople } from './people';
import { viewerStatus } from './viewer';
import type { Me, PersonRole, PersonState, PersonView, StatusView } from './types.gen';

const person = (id: string, role: PersonRole, state: PersonState = 'active'): PersonView => ({
  id,
  name: id,
  role,
  state,
  devices: [],
  passkeys: state === 'invited' ? 0 : 1,
  invited_by: state === 'active' ? null : 'ana',
  expires_at_ms: state === 'invited' ? 1 : null,
  me: false,
});
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

  it('lets only the owner change roles, of people in the ledger, never their own', () => {
    expect(roleChoices(as('owner'), admin)).toEqual(['member']);
    expect(roleChoices(as('owner'), member)).toEqual(['admin']);
    expect(roleChoices(as('owner'), owner)).toEqual([]);
    expect(roleChoices(as('owner'), { ...member, me: true })).toEqual([]);
    expect(roleChoices(as('owner'), person('di', 'member', 'registered'))).toEqual([]);
    expect(roleChoices(as('admin'), member)).toEqual([]);
  });

  it('lets admins remove members, the owner anyone but the owner, nobody themselves', () => {
    expect(canRemove(as('admin'), member)).toBe(true);
    expect(canRemove(as('admin'), admin)).toBe(false);
    expect(canRemove(as('owner'), admin)).toBe(true);
    expect(canRemove(as('owner'), owner)).toBe(false);
    expect(canRemove(as('member'), member)).toBe(false);
    expect(canRemove({ ...as('admin'), id: 'cy' }, member)).toBe(false);
    expect(canRemove(as('admin'), person('di', 'admin', 'invited'))).toBe(false);
  });

  it('asks whoever may add the person to approve them once they made a passkey', () => {
    const waiting = person('di', 'admin', 'registered');
    expect(canApprove(as('owner'), waiting)).toBe(true);
    expect(canApprove(as('admin'), waiting)).toBe(false);
    expect(canApprove(as('admin'), person('ed', 'member', 'registered'))).toBe(true);
    expect(canApprove(as('owner'), person('fy', 'member', 'invited'))).toBe(false);
  });

  it('lists people waiting first, then the owner, admins and members by name, then invites', () => {
    const list = [
      person('zed', 'member'),
      person('al', 'member', 'invited'),
      member,
      admin,
      owner,
      person('di', 'admin', 'registered'),
    ];
    expect(sortPeople(list).map((p) => p.id)).toEqual(['di', 'ana', 'bo', 'cy', 'zed', 'al']);
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
