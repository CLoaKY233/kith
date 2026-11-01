import { describe, expect, it } from 'vitest';
import { parseBoot, readInlineBoot } from './boot';
import { bootAt } from './testing';

describe('boot snapshot', () => {
  it('parses the inline snapshot the server writes', () => {
    const boot = bootAt(1_800_000_000_000);
    const text = JSON.stringify(boot);
    expect(parseBoot(text)).toEqual(boot);
    expect(readInlineBoot({ getElementById: () => ({ textContent: text }) as HTMLElement })).toEqual(boot);
    expect(parseBoot(JSON.stringify({ ...boot, last_tick: undefined }))?.last_tick).toBeNull();
  });

  it('rejects anything else, so the store falls back to /api/boot', () => {
    const boot = bootAt(1_800_000_000_000);
    const broken: unknown[] = [
      null,
      [],
      { ...boot, boot: '' },
      { ...boot, seq: -1 },
      { ...boot, seq: '4' },
      { ...boot, peers: { net: {} } },
      { ...boot, pending: { joins: [] } },
      { ...boot, status: { ...boot.status, networks: null } },
      { ...boot, versions: {} },
      { ...boot, last_tick: { traffic: [] } },
    ];
    for (const value of broken) expect(parseBoot(JSON.stringify(value))).toBeNull();
    expect(parseBoot('{"boot":')).toBeNull();
    expect(parseBoot('')).toBeNull();
    expect(readInlineBoot({ getElementById: () => null })).toBeNull();
    expect(readInlineBoot(null)).toBeNull();
  });
});
