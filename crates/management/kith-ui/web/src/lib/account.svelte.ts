// The person signed in to the Console (docs/plans/console.md): with a passkey, in public
// mode or on `localhost`. The local one-time link signs nobody in and none of this shows.

import { api } from './api';
import { live } from './live/store.svelte';
import type { ConsoleMode, Me } from './types.gen';

/** Who is signed in; null for the local link's session. */
export function me(): Me | null {
  return live.status?.me ?? null;
}

export function consoleMode(): ConsoleMode {
  return live.status?.mode ?? 'local';
}

const LOOPBACK = new Set(['127.0.0.1', 'localhost', '[::1]', '::1']);

class SignedOut {
  /** The mode, once asked; null while the question is out. */
  mode = $state<ConsoleMode | null>(null);
  #asked = false;

  /**
   * Without a session the snapshot can't say which mode this is, so ask `/api/status`,
   * which a public Console answers signed out; when that refuses too, a loopback host is
   * the local `kith ui`.
   */
  ask(): void {
    if (this.#asked) return;
    this.#asked = true;
    const guess: ConsoleMode = LOOPBACK.has(location.hostname) ? 'local' : 'public';
    api
      .get<{ mode?: ConsoleMode }>('/api/status')
      .then((status) => (this.mode = status.mode ?? guess))
      .catch(() => (this.mode = guess));
  }
}

export const signedOut = new SignedOut();

/** Ends this browser's session and goes to the passkey sign-in page. */
export async function signOut(): Promise<void> {
  try {
    await api.post('/api/logout');
  } finally {
    location.assign('/login');
  }
}
