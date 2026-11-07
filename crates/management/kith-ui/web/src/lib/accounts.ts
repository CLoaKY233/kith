import { api } from './api';
import type {
  AuthorityChallenge,
  AuthorityChangeIn,
  MeView,
  Outcome,
  PersonView,
  RecoveryWords,
  RegisterInfo,
  SetupFinished,
  SetupInfo,
} from './types.gen';
import type { AssertionJSON, CreationOptionsJSON, RegistrationJSON, RequestOptionsJSON } from './webauthn';

/** A generated type with its untyped `public_key` read as the options it holds. */
type Options<T, O> = Omit<T, 'public_key'> & { readonly public_key: O };

export type Ceremony<O> = { readonly public_key: O };
export type Authority = Options<AuthorityChallenge, RequestOptionsJSON>;
export type Setup = Omit<SetupFinished, 'authority'> & { readonly authority: Authority };

const token = (path: string, value: string): string => `${path}/${encodeURIComponent(value)}`;

/**
 * The Console's accounts (docs/plans/console.md, kith-ui-api.md "Console"). Setup, sign-in
 * and registration run before there is a session, so they are plain calls rather than
 * store actions.
 */
export const accounts = {
  setupInfo: (setup: string) => api.get<SetupInfo>(token('/api/setup', setup)),
  /** `networkName` only when the setup link named no network. */
  setupBegin: (setup: string, name: string, networkName: string | null) =>
    api.post<Ceremony<CreationOptionsJSON>>('/api/setup/begin', {
      token: setup,
      name,
      ...(networkName === null ? {} : { network_name: networkName }),
    }),
  /** Adds the owner and signs them in; `authority` authorizes this device as the Console. */
  setupFinish: (setup: string, credential: RegistrationJSON) =>
    api.post<Setup>('/api/setup/finish', { token: setup, credential }),
  /** The recovery words, once, to the owner setup added. */
  recovery: () => api.post<RecoveryWords>('/api/setup/recovery', {}),

  authorityBegin: (change: AuthorityChangeIn) => api.post<Authority>('/api/authority/begin', { change }),
  authorityFinish: (credential: AssertionJSON) => api.post<Outcome>('/api/authority/finish', { credential }),

  loginBegin: () => api.post<Ceremony<RequestOptionsJSON>>('/api/login/begin', {}),
  loginFinish: (credential: AssertionJSON) => api.post<MeView>('/api/login/finish', { credential }),

  registerInfo: (register: string) => api.get<RegisterInfo>(token('/api/register', register)),
  registerBegin: (register: string) =>
    api.post<Ceremony<CreationOptionsJSON>>('/api/register/begin', { token: register }),
  registerFinish: (register: string, credential: RegistrationJSON) =>
    api.post<PersonView>('/api/register/finish', { token: register, credential }),
};
