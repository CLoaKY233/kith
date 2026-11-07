// The only place the page calls fetch. Screens never call it directly; they go through
// the store's actions (§16 collision rules). Failures become an ApiError with the
// server's code, and errorText maps any failure to the copy of §12.5.

import { copy } from './copy';
import type { ApiErrorBody, ApiErrorCode, Conflict, NameProblem } from './types.gen';
import type { ApiFailure } from './types';

export class ApiError extends Error implements ApiFailure {
  readonly code: ApiErrorCode;
  /** The HTTP status; 0 when the request never got an answer. */
  readonly status: number;
  readonly conflict: Conflict | null;

  constructor(code: ApiErrorCode, status: number, message: string, conflict: Conflict | null) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
    this.conflict = conflict;
  }

  /** A plain copy, safe to keep in reactive state. */
  toFailure(): ApiFailure {
    return { code: this.code, status: this.status, message: this.message, conflict: this.conflict };
  }

  /** Wraps anything thrown into an ApiError. */
  static from(cause: unknown): ApiError {
    if (cause instanceof ApiError) return cause;
    return new ApiError('internal', 0, cause instanceof Error ? cause.message : String(cause), null);
  }
}

/** True for the error fetch throws when its signal aborts. */
export function isAbort(cause: unknown): boolean {
  return typeof cause === 'object' && cause !== null && (cause as { name?: unknown }).name === 'AbortError';
}

const CODES: readonly ApiErrorCode[] = [
  'unauthorized',
  'session_used',
  'session_expired',
  'forbidden',
  'permission_denied',
  'not_found',
  'conflict',
  'invalid_argument',
  'daemon_down',
  'not_in_group',
  'version_skew',
  'os_auth_denied',
  'rate_limited',
  'internal',
];

function isErrorBody(value: unknown): value is ApiErrorBody {
  if (typeof value !== 'object' || value === null) return false;
  const body = value as Record<string, unknown>;
  return (
    typeof body['code'] === 'string' &&
    CODES.includes(body['code'] as ApiErrorCode) &&
    typeof body['message'] === 'string'
  );
}

/** The code for an error answer whose body is not an ApiErrorBody (a proxy, a crash). */
const STATUS_CODES: Readonly<Record<number, ApiErrorCode>> = {
  400: 'invalid_argument',
  401: 'unauthorized',
  403: 'forbidden',
  404: 'not_found',
  409: 'conflict',
  410: 'session_expired',
  422: 'invalid_argument',
  429: 'rate_limited',
  503: 'daemon_down',
};

export type RequestOptions = { readonly signal?: AbortSignal };

async function request<T>(
  method: 'GET' | 'POST',
  path: string,
  body: unknown,
  options: RequestOptions,
): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      method,
      credentials: 'same-origin',
      headers: body === undefined ? {} : { 'content-type': 'application/json' },
      body: body === undefined ? null : JSON.stringify(body),
      signal: options.signal ?? null,
    });
  } catch (cause) {
    if (isAbort(cause)) throw cause;
    throw ApiError.from(cause);
  }
  const text = await response.text();
  let parsed: unknown = null;
  if (text !== '') {
    try {
      parsed = JSON.parse(text);
    } catch {
      parsed = null;
    }
  }
  if (!response.ok) {
    if (isErrorBody(parsed)) {
      throw new ApiError(parsed.code, response.status, parsed.message, parsed.conflict ?? null);
    }
    // A proxy page or a body this page does not know: never put it in a sentence.
    if (text !== '') console.warn(`kith-ui: ${method} ${path} answered ${response.status}`, text);
    const message = response.statusText || `HTTP ${response.status}`;
    throw new ApiError(STATUS_CODES[response.status] ?? 'internal', response.status, message, null);
  }
  return parsed as T;
}

export const api = {
  get: <T>(path: string, options: RequestOptions = {}): Promise<T> =>
    request<T>('GET', path, undefined, options),
  post: <T>(path: string, body: unknown = {}, options: RequestOptions = {}): Promise<T> =>
    request<T>('POST', path, body, options),
};

/** `/api/networks/:net` with the id escaped. */
export function networkPath(network: string, rest: string): string {
  return `/api/networks/${encodeURIComponent(network)}${rest}`;
}

/** What errorText needs besides the failure itself. */
export type ErrorContext = {
  /** The device or name the action was about, for `tv is no longer in this network.` */
  readonly subject?: string;
  /** Field-specific copy for an InvalidArgument (§12.5), when the screen has it. */
  readonly invalid?: string;
};

/**
 * §12.5: the sentence for a failed action, cause first, then the next step. A 409 with
 * its conflict names who changed it; screens with a more exact sentence (a join that
 * someone else approved) use `conflict` themselves.
 */
export function errorText(failure: ApiFailure, context: ErrorContext = {}): string {
  switch (failure.code) {
    case 'permission_denied':
      return copy.error.permissionDenied;
    case 'not_found':
      return context.subject ? copy.error.notFound(context.subject) : copy.error.notFoundPlain;
    case 'conflict':
      return failure.conflict
        ? copy.error.conflict(failure.conflict.resolved_by)
        : copy.error.conflictPlain;
    case 'invalid_argument':
      return context.invalid ?? copy.error.invalid(failure.message);
    case 'daemon_down':
      return copy.conn.staleActions;
    case 'os_auth_denied':
      return copy.error.osAuthDenied;
    case 'rate_limited':
      return copy.error.rateLimited;
    case 'unauthorized':
      return copy.conn.unauth;
    case 'session_used':
      return copy.gate.authUsed;
    case 'session_expired':
      return copy.gate.authExpired;
    case 'forbidden':
      return copy.gate.authTitle;
    case 'not_in_group':
      return copy.gate.groupTitle;
    case 'version_skew':
    case 'internal':
      return failure.status === 0 ? copy.conn.uiGone : copy.error.internal(failure.message);
  }
}

/** §7.11 and §6.6 field errors for a name the validator refused; `taken` needs context. */
export function nameProblemText(problem: Exclude<NameProblem, 'taken'>): string {
  switch (problem) {
    case 'empty':
      return copy.error.nameEmpty;
    case 'too_long':
      return copy.error.nameTooLong;
    case 'invalid_character':
      return copy.error.nameChars;
    case 'hyphen_at_edge':
      return copy.error.nameHyphenEdge;
  }
}
