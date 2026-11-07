import type { StatusView } from './types.gen';

/**
 * The status as this person may act on it. The Console's own device is an admin of the
 * network, so a member signed in through it must still get the member view everywhere.
 */
export function viewerStatus<S extends StatusView>(status: S): S {
  if (status.me?.role !== 'member') return status;
  return {
    ...status,
    networks: status.networks.map((n) => ({ ...n, admin: false })),
    memberships: status.memberships.map((m) => (m.role === 'admin' ? { ...m, role: 'member' } : m)),
  };
}
