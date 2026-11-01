// The only place the page calls fetch. Screens never call it directly; they go through
// the store's actions (§16 collision rules).

import type { ApiErrorBody, ApiErrorCode } from './types.gen';
import type { ApiFailure } from './types';

export class ApiError extends Error implements ApiFailure {
  readonly code: ApiErrorCode;
  readonly status: number;
  readonly conflict: ApiFailure['conflict'];

  constructor(code: ApiErrorCode, status: number, message: string, conflict: ApiFailure['conflict']) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
    this.conflict = conflict;
  }
}

function isErrorBody(value: unknown): value is ApiErrorBody {
  return typeof value === 'object' && value !== null && 'code' in value && 'message' in value;
}

const STATUS_CODES: Record<number, ApiErrorCode> = {
  401: 'unauthorized',
  403: 'forbidden',
  404: 'not_found',
  409: 'conflict',
  422: 'invalid_argument',
  501: 'not_implemented',
  503: 'daemon_down',
};

async function request<T>(method: 'GET' | 'POST', path: string, body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      method,
      credentials: 'same-origin',
      headers: body === undefined ? {} : { 'content-type': 'application/json' },
      body: body === undefined ? null : JSON.stringify(body),
    });
  } catch (cause) {
    throw new ApiError('internal', 0, cause instanceof Error ? cause.message : String(cause), null);
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
      throw new ApiError(parsed.code, response.status, parsed.message, parsed.conflict);
    }
    throw new ApiError(STATUS_CODES[response.status] ?? 'internal', response.status, text, null);
  }
  return parsed as T;
}

export const api = {
  get: <T>(path: string): Promise<T> => request<T>('GET', path),
  post: <T>(path: string, body: unknown = {}): Promise<T> => request<T>('POST', path, body),
};

/** `/api/networks/:net` with the id escaped. */
export function networkPath(network: string, rest: string): string {
  return `/api/networks/${encodeURIComponent(network)}${rest}`;
}
