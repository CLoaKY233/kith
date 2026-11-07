import { api } from './api';
import type { AuthorityChange, Me } from './types';
import type { AssertionJSON, CreationOptionsJSON, RegistrationJSON, RequestOptionsJSON } from './webauthn';

/**
 * The Console's accounts (docs/plans/console.md). Setup and sign-in run before there is a
 * session, so they are plain calls rather than store actions.
 */
export const accounts = {
  setupBegin: (token: string, networkName: string, personName: string) =>
    api.post<{ publicKey: CreationOptionsJSON }>('/api/setup/begin', {
      token,
      network_name: networkName,
      person_name: personName,
    }),
  setupFinish: (token: string, credential: RegistrationJSON) =>
    api.post<{ authority: RequestOptionsJSON }>('/api/setup/finish', { token, credential }),
  /** Signs a prepared authority op; the first one, at setup, answers the recovery words. */
  authorityFinish: (assertion: AssertionJSON) =>
    api.post<{ recovery_words?: string[] }>('/api/authority/finish', { assertion }),
  authorityBegin: (change: AuthorityChange) =>
    api.post<{ publicKey: RequestOptionsJSON }>('/api/authority/begin', { change }),
  loginBegin: () => api.post<{ publicKey: RequestOptionsJSON }>('/api/login/begin', {}),
  loginFinish: (assertion: AssertionJSON) => api.post<{ person: Me }>('/api/login/finish', { assertion }),
};
