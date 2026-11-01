import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { EASING, MOTION } from './live/timing';

const css = readFileSync(new URL('./tokens.css', import.meta.url), 'utf8');
const rootBlock = css.slice(css.indexOf(':root {'), css.indexOf('/* §3.2'));

function token(block: string, name: string): string | undefined {
  return new RegExp(`--${name}:\\s*([^;]+);`).exec(block)?.[1]?.trim();
}

const kebab = (key: string): string => key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);

describe('tokens.css', () => {
  it('mirrors every timing.ts motion duration and easing', () => {
    for (const [key, ms] of Object.entries(MOTION)) {
      expect(token(rootBlock, `d-${kebab(key)}`), key).toBe(`${ms}ms`);
    }
    for (const [key, value] of Object.entries(EASING)) {
      expect(token(rootBlock, `e-${key}`), key).toBe(value);
    }
  });

  it('has the exact light and dark colours of spec §3.1 and §3.2', () => {
    expect(token(rootBlock, 'accent')).toBe('#2f62d0');
    expect(token(rootBlock, 'attn')).toBe('#c87a12');
    expect(token(rootBlock, 'danger')).toBe('#c8322b');
    const dark = css.slice(css.indexOf(":root[data-theme='dark']"));
    expect(token(dark, 'accent')).toBe('#6a95ec');
    expect(token(dark, 'bg')).toBe('#111316');
    expect(token(dark, 'changed')).toBe('rgb(106 149 236 / 0.1)');
  });

  it('defines dark mode identically for the media query and data-theme', () => {
    const media = css.slice(css.indexOf(":root:not([data-theme='light'])"), css.indexOf(":root[data-theme='dark']"));
    const forced = css.slice(css.indexOf(":root[data-theme='dark']"), css.indexOf('/* Touch'));
    const decls = (block: string): string[] =>
      [...block.matchAll(/--[\w-]+:\s*[^;]+;/g)].map((m) => m[0]);
    expect(decls(media)).toEqual(decls(forced));
  });

  it('uses only the spacing scale of spec §3.4', () => {
    const steps = [4, 8, 12, 16, 24, 32, 48, 64];
    steps.forEach((px, i) => expect(token(rootBlock, `s-${i + 1}`)).toBe(`${px}px`));
  });
});
