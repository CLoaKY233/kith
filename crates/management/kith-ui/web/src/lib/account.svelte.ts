// The person signed in to the Console (docs/plans/console.md). In local mode nobody signs
// in and none of this shows.

import { api } from './api';
import { live } from './live/store.svelte';
import type { ConsoleMode, ConsoleStatus, Me } from './types';

function consoleStatus(): ConsoleStatus | null {
  return live.status as ConsoleStatus | null;
}

/** Who is signed in; null in local mode. */
export function me(): Me | null {
  return consoleStatus()?.me ?? null;
}

export function consoleMode(): ConsoleMode {
  return consoleStatus()?.mode ?? 'local';
}

const LOOPBACK = new Set(['127.0.0.1', 'localhost', '[::1]', '::1']);

class SignedOut {
  /** The mode, once asked; null while the question is out. */
  mode = $state<ConsoleMode | null>(null);
  #asked = false;

  /**
   * Without a session the snapshot can't say which mode this is, so ask `/api/status`;
   * when that refuses too, a loopback host is the local `kith ui`.
   */
  ask(): void {
    if (this.#asked) return;
    this.#asked = true;
    const guess: ConsoleMode = LOOPBACK.has(location.hostname) ? 'local' : 'public';
    api
      .get<ConsoleStatus>('/api/status')
      .then((status) => (this.mode = status.mode ?? guess))
      .catch(() => (this.mode = guess));
  }
}

export const signedOut = new SignedOut();

export async function signOut(): Promise<void> {
  try {
    await api.post('/api/logout');
  } finally {
    location.assign('/');
  }
}
