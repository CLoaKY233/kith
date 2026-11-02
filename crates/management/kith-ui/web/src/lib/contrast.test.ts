// Spec §11.3: every colour pair the page uses meets WCAG 2.2 contrast in both themes,
// text at 4.5:1 (1.4.3) and non-text marks at 3:1 (1.4.11). The pairs are the ones the
// components draw; adding a new pairing to a component means adding it here.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const css = readFileSync(new URL('./tokens.css', import.meta.url), 'utf8');

function block(start: string, end: string): string {
  const from = css.indexOf(start);
  const to = css.indexOf(end, from);
  if (from < 0 || to < 0) throw new Error(`tokens.css has no block ${start}`);
  return css.slice(from, to);
}

const THEMES = {
  light: block(':root {', '/* §3.2'),
  dark: block(":root[data-theme='dark']", '/* Touch'),
} as const;

function hex(theme: string, name: string): string {
  const value = new RegExp(`--${name}:\\s*(#[0-9a-f]{6});`, 'i').exec(theme)?.[1];
  if (value === undefined) throw new Error(`--${name} is not a hex colour in this theme`);
  return value;
}

function luminance(colour: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = Number.parseInt(colour.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function ratio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

const PAGES = ['bg', 'surface'];
const FILLS = [...PAGES, 'sunken'];
const WASHES = ['attn-wash', 'danger-wash'];

/** Foreground on background, for text. */
const TEXT: [string, string[]][] = [
  ['ink-1', [...FILLS, ...WASHES, 'line']],
  ['ink-2', [...FILLS, ...WASHES]],
  // Meta and offline text, also on selected and hovered rows and on the amber wash.
  ['ink-3', [...FILLS, 'attn-wash']],
  ['accent', FILLS],
  ['attn-text', [...PAGES, 'attn-wash']],
  ['danger', [...FILLS, 'danger-wash']],
  // Filled buttons and the tooltip.
  ['accent-fg', ['accent']],
  ['accent-fg', ['danger']],
  ['surface', ['ink-1']],
];

/** Marks that identify a control or a state: borders, dots, rings, the focus ring. */
const NON_TEXT: [string, string[]][] = [
  // Input, toggle and outline-chip borders against the page they sit on, and the selected
  // segment's border against its own --surface fill.
  ['line-strong', PAGES],
  ['accent', [...FILLS, 'attn-wash']],
  ['ink-1', [...FILLS, 'attn-wash']],
  // The offline ring and the switch's off thumb on its track.
  ['ink-3', [...FILLS, 'attn-wash']],
  ['attn', [...FILLS, 'attn-wash']],
  ['danger', [...FILLS, 'danger-wash']],
  // The switch's on thumb.
  ['accent-fg', ['accent']],
];

describe.each(Object.entries(THEMES))('contrast, %s theme', (_, theme) => {
  it.each(TEXT)('%s text reaches 4.5:1', (fg, backgrounds) => {
    for (const bg of backgrounds) {
      expect(ratio(hex(theme, fg), hex(theme, bg)), `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it.each(NON_TEXT)('%s marks reach 3:1', (fg, backgrounds) => {
    for (const bg of backgrounds) {
      expect(ratio(hex(theme, fg), hex(theme, bg)), `${fg} on ${bg}`).toBeGreaterThanOrEqual(3);
    }
  });
});

describe('contrast maths', () => {
  it('matches the WCAG reference values', () => {
    expect(ratio('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(ratio('#777777', '#ffffff')).toBeCloseTo(4.48, 2);
  });
});
