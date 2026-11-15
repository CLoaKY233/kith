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
} from '@kith/types';
import { api } from './api';
import type { AssertionJSON, CreationOptionsJSON, RegistrationJSON, RequestOptionsJSON } from './webauthn';

type Ceremony<O> = { readonly public_key: O };
type Authority = Omit<AuthorityChallenge, 'public_key'> & { readonly public_key: RequestOptionsJSON };
type Setup = Omit<SetupFinished, 'authority'> & { readonly authority: Authority };

const token = (path: string, value: string) => `${path}/${encodeURIComponent(value)}`;

/** The Console's accounts. Setup, sign-in and registration run before there is a session. */
export const accounts = {
  setupInfo: (setup: string) => api.get<SetupInfo>(token('/api/setup', setup)),
  /** `networkName` only when the setup link named no network. */
  setupBegin: (setup: string, name: string, networkName: string | null) =>
    api.post<Ceremony<CreationOptionsJSON>>('/api/setup/begin', {
      token: setup,
      name,
      ...(networkName === null ? {} : { network_name: networkName }),
    }),
  setupFinish: (setup: string, credential: RegistrationJSON) =>
    api.post<Setup>('/api/setup/finish', { token: setup, credential }),
  recovery: () => api.post<RecoveryWords>('/api/setup/recovery', {}),

  authorityBegin: (change: AuthorityChangeIn) => api.post<Authority>('/api/authority/begin', { change }),
  authorityFinish: (credential: AssertionJSON) => api.post<Outcome>('/api/authority/finish', { credential }),

  passkeyBegin: () => api.post<Ceremony<CreationOptionsJSON>>('/api/passkeys/begin', {}),
  passkeyFinish: (credential: RegistrationJSON) => api.post<Authority>('/api/passkeys/finish', { credential }),

  recoverBegin: (name: string) => api.post<Ceremony<CreationOptionsJSON>>('/api/recover/begin', { name }),
  recoverFinish: (credential: RegistrationJSON, words: string) =>
    api.post<{ account: string }>('/api/recover/finish', { credential, words }),

  loginBegin: () => api.post<Ceremony<RequestOptionsJSON>>('/api/login/begin', {}),
  loginFinish: (credential: AssertionJSON) => api.post<MeView>('/api/login/finish', { credential }),

  registerInfo: (register: string) => api.get<RegisterInfo>(token('/api/register', register)),
  registerBegin: (register: string) =>
    api.post<Ceremony<CreationOptionsJSON>>('/api/register/begin', { token: register }),
  registerFinish: (register: string, credential: RegistrationJSON) =>
    api.post<PersonView>('/api/register/finish', { token: register, credential }),
};
