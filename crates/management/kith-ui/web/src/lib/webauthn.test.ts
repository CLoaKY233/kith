import { describe, expect, it } from 'vitest';
import { creationOptions, fromBase64url, passkeyProblem, requestOptions, toBase64url } from './webauthn';

const bytes = (...values: number[]): Uint8Array => new Uint8Array(values);

describe('base64url', () => {
  it('encodes without padding and with the URL-safe alphabet', () => {
    expect(toBase64url(bytes())).toBe('');
    expect(toBase64url(bytes(0xfb, 0xff))).toBe('-_8');
    expect(toBase64url(new TextEncoder().encode('kith'))).toBe('a2l0aA');
    expect(toBase64url(bytes(1, 2, 3).buffer as ArrayBuffer)).toBe('AQID');
  });

  it('round-trips every byte value, at every length mod 3', () => {
    const all = Uint8Array.from({ length: 256 }, (_, i) => i);
    for (const n of [254, 255, 256]) {
      const slice = all.slice(0, n);
      expect(fromBase64url(toBase64url(slice))).toEqual(slice);
    }
  });

  it('reads a view into a larger buffer as only its own bytes', () => {
    const buffer = bytes(9, 9, 1, 2, 3, 9).buffer;
    expect(toBase64url(new Uint8Array(buffer, 2, 3))).toBe('AQID');
  });

  it('accepts padding and refuses anything that is not base64url', () => {
    expect(fromBase64url('a2l0aA==')).toEqual(new TextEncoder().encode('kith'));
    for (const bad of ['a+b/', 'a b', 'abcde', '=abc']) {
      expect(() => fromBase64url(bad), bad).toThrow(TypeError);
    }
  });
});

describe('options from the server', () => {
  it('decodes every binary field of creation options', () => {
    const options = creationOptions({
      rp: { id: 'vpn.example.com', name: 'kith' },
      user: { id: 'AQID', name: 'ana', displayName: 'Ana' },
      challenge: '-_8',
      pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
      excludeCredentials: [{ id: 'AAE', type: 'public-key', transports: ['internal'] }],
      authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
    });
    expect(options.user.id).toEqual(bytes(1, 2, 3));
    expect(options.challenge).toEqual(bytes(0xfb, 0xff));
    expect(options.excludeCredentials?.[0]?.id).toEqual(bytes(0, 1));
    expect(options.excludeCredentials?.[0]?.transports).toEqual(['internal']);
    expect(options.rp).toEqual({ id: 'vpn.example.com', name: 'kith' });
    expect('timeout' in options).toBe(false);
  });

  it('decodes request options and leaves out what the server did not send', () => {
    const options = requestOptions({ challenge: 'AQID', userVerification: 'required' });
    expect(options.challenge).toEqual(bytes(1, 2, 3));
    expect(options.userVerification).toBe('required');
    expect('allowCredentials' in options).toBe(false);
    expect(requestOptions({ challenge: '', allowCredentials: [{ id: 'AQ', type: 'public-key' }] }).allowCredentials?.[0]?.id).toEqual(bytes(1));
  });
});

describe('passkeyProblem', () => {
  it('reads a closed prompt as cancelled and names the rest', () => {
    const named = (name: string): { name: string } => ({ name });
    expect(passkeyProblem(named('NotAllowedError'))).toBe('cancelled');
    expect(passkeyProblem(named('AbortError'))).toBe('cancelled');
    expect(passkeyProblem(named('InvalidStateError'))).toBe('exists');
    expect(passkeyProblem(named('SecurityError'))).toBe('unsupported');
    expect(passkeyProblem(new Error('boom'))).toBe('failed');
  });
});
