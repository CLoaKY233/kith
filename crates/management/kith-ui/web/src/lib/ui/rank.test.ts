import { describe, expect, it } from 'vitest';
import { parseQuery, rank, remember, score } from './rank';

const items = [
  { id: 'nas', label: 'nas', keywords: ['100.64.0.2'] },
  { id: 'laptop', label: 'laptop', keywords: ['100.64.0.3'] },
  { id: 'add', label: 'Add device' },
  { id: 'rename', label: 'Rename nas' },
];

describe('palette queries (spec §10)', () => {
  it('reads the scope prefix and a name:port', () => {
    expect(parseQuery('> add')).toEqual({ scope: 'actions', text: 'add' });
    expect(parseQuery('@nas')).toEqual({ scope: 'devices', text: 'nas' });
    expect(parseQuery('nas:22')).toEqual({ scope: 'all', text: 'nas', port: 22 });
    expect(parseQuery('@nas:443')).toEqual({ scope: 'devices', text: 'nas', port: 443 });
    expect(parseQuery('nas:99999')).toEqual({ scope: 'all', text: 'nas:99999' });
    expect(parseQuery('>a:1')).toEqual({ scope: 'actions', text: 'a:1' });
  });

  it('scores prefixes over word starts over substrings over letters in order', () => {
    const prefix = score('nas-backup', 'nas');
    const word = score('old nas', 'nas');
    const inside = score('dynasty', 'nas');
    const letters = score('north-america-south', 'nas');
    expect(prefix).toBeGreaterThan(word);
    expect(word).toBeGreaterThan(inside);
    expect(inside).toBeGreaterThan(letters);
    expect(letters).toBeGreaterThan(0);
    expect(score('laptop', 'nas')).toBe(0);
    expect(score('Rename vps', 'nas')).toBe(0);
  });

  it('ranks recent picks first, then by match, keeping ties in order', () => {
    expect(rank(items, 'nas', []).map((i) => i.id)).toEqual(['nas', 'rename']);
    expect(rank(items, 'nas', ['rename']).map((i) => i.id)).toEqual(['rename', 'nas']);
    expect(rank(items, '100.64.0.3', []).map((i) => i.id)).toEqual(['laptop']);
    expect(rank(items, '', ['add']).map((i) => i.id)).toEqual(['add', 'nas', 'laptop', 'rename']);
    const loose = [
      { id: 'weak', label: 'device' },
      { id: 'strong', label: 'dvc box' },
    ];
    expect(rank(loose, 'dvc', ['weak']).map((i) => i.id)).toEqual(['strong', 'weak']);
  });

  it('keeps the recency list unique and bounded', () => {
    expect(remember(['a', 'b', 'c'], 'c', 3)).toEqual(['c', 'a', 'b']);
    expect(remember(['a', 'b', 'c'], 'd', 3)).toEqual(['d', 'a', 'b']);
  });
});
