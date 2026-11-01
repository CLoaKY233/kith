import { describe, expect, it } from 'vitest';
import { copy } from './copy';

/** Every string in copy.ts, with functions called on sample arguments. */
function strings(value: unknown, path: string, out: [string, string][]): [string, string][] {
  if (typeof value === 'string') out.push([path, value]);
  else if (typeof value === 'function') {
    const call = value as (...args: unknown[]) => unknown;
    const samples: unknown[][] = [
      ['nas', 'home', 'vps', 'x', 'y'],
      [2, 3, 4, 5, 6],
      [['desk', 'nas']],
    ];
    for (const [i, args] of samples.entries()) {
      try {
        strings(call(...args), `${path}#${i}`, out);
      } catch {
        // A template that needs other argument types; another sample covers it.
      }
    }
  } else if (Array.isArray(value)) value.forEach((v, i) => strings(v, `${path}[${i}]`, out));
  else if (value && typeof value === 'object') {
    for (const [key, v] of Object.entries(value)) strings(v, `${path}.${key}`, out);
  }
  return out;
}

const all = strings(copy, 'copy', []);

describe('copy.ts (spec §12.1)', () => {
  it('has no banned words or punctuation', () => {
    const banned = [/!/, /successfully/i, /\bplease\b/i, /\boops\b/i, /Error:/, /Loading…/];
    for (const [path, text] of all) {
      for (const pattern of banned) expect(text, path).not.toMatch(pattern);
    }
  });

  it('uses the vocabulary of spec §12.2', () => {
    const never = [/\bpeers?\b/i, /\bmesh\b/i, /\bexit node\b/i, /\bjournal\b/i];
    for (const [path, text] of all) {
      for (const pattern of never) expect(text, path).not.toMatch(pattern);
      // "dropped" is never a blocked connection; §7.9's "have dropped phone" is a verb.
      if (!path.startsWith('copy.remove.')) expect(text, path).not.toMatch(/\bdropped\b/i);
    }
  });

  it('spells check codes out for screen readers', () => {
    expect(copy.spoken.checkCode('K7Q·4XM')).toBe('K 7 Q, 4 X M');
  });

  it('prefixes the title with unseen changes', () => {
    expect(copy.documentTitle('laptop', 'home', 2)).toBe('(2) laptop · home — kith');
    expect(copy.documentTitle('laptop', 'home', 0)).toBe('laptop · home — kith');
  });
});
