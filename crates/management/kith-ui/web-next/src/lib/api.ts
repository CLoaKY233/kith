// Every call to the kith API goes through here.
type Code =
  | 'unauthorized'
  | 'session_used'
  | 'session_expired'
  | 'forbidden'
  | 'permission_denied'
  | 'not_found'
  | 'conflict'
  | 'invalid_argument'
  | 'daemon_down'
  | 'not_in_group'
  | 'version_skew'
  | 'os_auth_denied'
  | 'rate_limited'
  | 'internal'
  | 'offline';

/** What a person reads for each kind of failure; the server's own words stay in `detail`. */
const WORDS: Record<Code, string> = {
  unauthorized: 'You are signed out. Sign in again.',
  session_used: 'This link was already used.',
  session_expired: 'This link has expired.',
  forbidden: 'This page cannot be opened from this address.',
  permission_denied: 'You do not have permission to do that.',
  not_found: 'That is no longer in this network.',
  conflict: 'Someone else changed this first. Reload and try again.',
  invalid_argument: 'That value is not valid.',
  daemon_down: 'kith is not responding. Try again in a moment.',
  not_in_group: 'This user cannot talk to kith. Ask an admin to add you to the kith group.',
  version_skew: 'kith needs a restart to match this page. Restart it and reload.',
  os_auth_denied: 'The confirmation was cancelled.',
  rate_limited: 'Too many tries. Wait a minute and try again.',
  internal: 'Something went wrong. Try again.',
  offline: 'Cannot reach kith. Check your connection.',
};

const STATUS_CODES: Readonly<Record<number, Code>> = {
  400: 'invalid_argument',
  401: 'unauthorized',
  403: 'forbidden',
  404: 'not_found',
  409: 'conflict',
  410: 'session_expired',
  422: 'invalid_argument',
  429: 'rate_limited',
  502: 'daemon_down',
  503: 'daemon_down',
  504: 'daemon_down',
};

const isCode = (v: unknown): v is Code => typeof v === 'string' && v in WORDS;

/** `message` is always a short sentence fit for a toast; `detail` is the raw cause, for titles and logs. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly detail: string = message,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

/** Anything thrown becomes a sentence a person can read. */
export const errorMessage = (cause: unknown) =>
  cause instanceof ApiError ? cause.message : WORDS.internal;

async function call<T>(method: 'GET' | 'POST', path: string, body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      method,
      credentials: 'same-origin',
      cache: 'no-store',
      headers: body === undefined ? {} : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (cause) {
    throw new ApiError(0, 'offline', WORDS.offline, String(cause));
  }
  const text = await response.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    // A proxy page or a crash: never shown to people.
  }
  if (!response.ok) {
    const body = (data ?? {}) as { code?: unknown; message?: unknown };
    const code: Code = isCode(body.code) ? body.code : (STATUS_CODES[response.status] ?? 'internal');
    const detail = typeof body.message === 'string' && body.message ? body.message : response.statusText || `HTTP ${response.status}`;
    if (!isCode(body.code)) console.warn(`kith: ${method} ${path} answered ${response.status}`, text.slice(0, 500));
    // An invalid value is explained in the server's own short sentence; anything long or technical stays in `detail`.
    const said = code === 'invalid_argument' && typeof body.message === 'string' ? body.message.trim() : '';
    const message = said && said.length <= 120 && !/[\n{}<>]/.test(said) ? said : WORDS[code];
    throw new ApiError(response.status, code, message, detail);
  }
  if (text && data === null) throw new ApiError(response.status, 'internal', WORDS.internal, 'Unreadable answer');
  return data as T;
}

export const api = {
  get: <T>(path: string) => call<T>('GET', path),
  post: <T>(path: string, body?: unknown) => call<T>('POST', path, body),
};

export const net = (id: string, rest = '') => `/api/networks/${encodeURIComponent(id)}${rest}`;

/**
 * Trades the one-time `#t=` link token for a session cookie, then drops it from the URL.
 * Answers why the link was refused when it was used or expired.
 */
export async function startSession(): Promise<'session_used' | 'session_expired' | null> {
  const match = /[#&]t=([0-9a-f]+)/.exec(location.hash);
  if (!match) return null;
  history.replaceState(null, '', location.pathname + location.search);
  try {
    await api.post('/api/session', { token: match[1] });
    return null;
  } catch (cause) {
    const code = cause instanceof ApiError ? cause.code : null;
    return code === 'session_used' || code === 'session_expired' ? code : null;
  }
}
