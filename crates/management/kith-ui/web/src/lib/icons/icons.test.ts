import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ICONS } from './names';

const sprite = readFileSync(new URL('./sprite.svg', import.meta.url), 'utf8');
const symbols = [...sprite.matchAll(/<symbol id="i-([\w-]+)" viewBox="0 0 24 24">/g)].map((m) => m[1]);

describe('icon sprite (spec §5 Icons)', () => {
  it('has exactly the names in names.ts, each on the 24-unit grid', () => {
    expect(symbols).toEqual([...ICONS]);
  });

  it('covers every icon the spec names, plus the custom glyphs', () => {
    const spec = [
      'activity', 'monitor-smartphone', 'list', 'at-sign', 'globe', 'plus', 'settings-2',
      'inbox', 'search', 'copy', 'check', 'ellipsis', 'ban', 'triangle-alert', 'circle-x',
      'qr-code', 'ticket', 'laptop', 'monitor', 'smartphone', 'server', 'cpu', 'key-round',
      'radio-tower', 'path-direct', 'path-relayed', 'path-stealth', 'seal', 'mark',
    ];
    for (const name of spec) expect(symbols, name).toContain(name);
  });

  it('leaves stroke and fill to Icon.svelte, so every use inherits the 1.5 stroke', () => {
    expect(sprite).not.toMatch(/stroke-width=/);
    expect(sprite).not.toMatch(/\bfill="(?!none|currentColor)/);
  });
});
