import { describe, expect, it } from 'vitest';
import { normaliseName, recognise } from './flow';

describe('recognise', () => {
  it('takes an invite link, also inside a pasted command', () => {
    const link = 'kith://join/mfrggzdfmzt';
    expect(recognise(`  ${link} `)).toEqual({ kind: 'invite', link });
    expect(recognise(`kith join ${link}`)).toEqual({ kind: 'invite', link });
    expect(recognise('KITH://JOIN/MFRGG')).toEqual({ kind: 'invite', link: 'KITH://JOIN/MFRGG' });
  });

  it('turns network@admin and request links into a request', () => {
    expect(recognise('kith join Home@Studio-Mac')).toEqual({
      kind: 'request',
      code: 'home@studio-mac',
      network: 'home',
      admin: 'studio-mac',
    });
    expect(recognise('kith://request/abc')).toEqual({
      kind: 'request',
      code: 'kith://request/abc',
      network: null,
      admin: null,
    });
  });

  it('refuses anything else', () => {
    for (const text of ['', 'hello', 'kith://join/', 'https://kith.sh', 'home@', 'a b@c']) {
      expect(recognise(text)).toBeNull();
    }
  });
});

describe('normaliseName', () => {
  it('lowers capitals and makes spaces hyphens, saying when it did', () => {
    expect(normaliseName('My Net')).toEqual({ value: 'my-net', spaced: true });
    expect(normaliseName('home')).toEqual({ value: 'home', spaced: false });
  });
});
