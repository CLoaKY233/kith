import type { ConsoleMode } from '@kith/types';
import { api } from './api';

const LOOPBACK = new Set(['127.0.0.1', 'localhost', '[::1]', '::1']);

/**
 * Which Console this is, asked without a session: a public Console answers `/api/status`
 * signed out; when that refuses too, a loopback host is the local `kith ui`.
 */
export async function consoleMode(): Promise<ConsoleMode> {
  const guess: ConsoleMode = LOOPBACK.has(location.hostname) ? 'local' : 'public';
  try {
    return (await api.get<{ mode?: ConsoleMode }>('/api/status')).mode ?? guess;
  } catch {
    return guess;
  }
}

/** Ends this browser's session and goes to the passkey sign-in page. */
export async function signOut(): Promise<void> {
  try {
    await api.post('/api/logout');
  } catch {
    // The session cookie may already be gone; the sign-in page is the right place either way.
  } finally {
    location.assign('/login');
  }
}
