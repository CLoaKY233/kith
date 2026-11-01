import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError, errorText, nameProblemText } from './api';
import { copy } from './copy';

function answer(status: number, body: string): void {
  vi.stubGlobal('fetch', vi.fn(async () => new Response(body === '' ? null : body, { status })));
}

async function failure(promise: Promise<unknown>): Promise<ApiError> {
  try {
    await promise;
  } catch (cause) {
    if (cause instanceof ApiError) return cause;
    throw cause;
  }
  throw new Error('expected the request to fail');
}

afterEach(() => vi.unstubAllGlobals());

describe('api (spec §2)', () => {
  it('returns the JSON body of a 2xx', async () => {
    answer(200, '{"state":"accepted","op":null}');
    await expect(api.post('/api/exit', {})).resolves.toEqual({ state: 'accepted', op: null });
  });

  it('keeps the code and conflict of an ApiErrorBody', async () => {
    const conflict = { resolved_by: 'laptop', resolution: 'approved', at_ms: 5, op: null };
    answer(409, JSON.stringify({ code: 'conflict', message: 'resolved', conflict }));
    const error = await failure(api.post('/api/networks/h/joins/n/approve'));
    expect(error.code).toBe('conflict');
    expect(error.status).toBe(409);
    expect(error.conflict).toEqual(conflict);
  });

  it('maps a bare status to a code when the body is not ours', async () => {
    answer(503, 'Service Unavailable');
    expect((await failure(api.get('/api/status'))).code).toBe('daemon_down');
    answer(502, '');
    expect((await failure(api.get('/api/status'))).code).toBe('internal');
  });

  it('reports a request that never got an answer with status 0', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => Promise.reject(new TypeError('Failed to fetch'))));
    const error = await failure(api.get('/api/status'));
    expect(error.status).toBe(0);
    expect(errorText(error)).toBe(copy.conn.uiGone);
  });
});

describe('errorText (spec §12.5)', () => {
  const f = (code: ApiError['code'], message = 'boom', status = 500) =>
    new ApiError(code, status, message, null).toFailure();

  it('maps every code to its copy, cause first', () => {
    expect(errorText(f('permission_denied'))).toBe(copy.error.permissionDenied);
    expect(errorText(f('not_found'), { subject: 'tv' })).toBe('tv is no longer in this network.');
    expect(errorText(f('internal', 'disk full'))).toBe(
      "kith couldn't do this: disk full. Details are in Logs.",
    );
    expect(errorText(f('daemon_down'))).toBe(copy.conn.staleActions);
    expect(errorText(f('os_auth_denied'))).toBe(copy.error.osAuthDenied);
    expect(errorText(f('invalid_argument'), { invalid: copy.error.nameChars })).toBe(
      copy.error.nameChars,
    );
    const conflict = new ApiError('conflict', 409, 'x', {
      resolved_by: 'laptop',
      resolution: null,
      at_ms: 1,
      op: 'op1',
    }).toFailure();
    expect(errorText(conflict)).toBe('laptop changed this a moment ago. Showing the latest.');
  });

  it('maps validator problems to field errors', () => {
    expect(nameProblemText('invalid_character')).toBe('Names can use a–z, 0–9 and hyphens.');
    expect(nameProblemText('empty')).toBe(copy.error.nameEmpty);
  });
});
