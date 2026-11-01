// The canvas mirror of tokens.css. Canvas and SVG painters cannot use CSS variables, so
// they read the resolved values here, refreshed whenever the theme changes. No colour is
// written in this file: every value comes from getComputedStyle.

/** Every colour token of spec §3.1 and §3.2, by its CSS name. */
export const COLOR_TOKENS = [
  'bg',
  'surface',
  'sunken',
  'line',
  'line-strong',
  'ink-1',
  'ink-2',
  'ink-3',
  'ink-4',
  'accent',
  'accent-fg',
  'attn',
  'attn-text',
  'attn-wash',
  'danger',
  'danger-wash',
  'changed',
  'scrim',
  'chart-ink',
] as const;

export type ColorToken = (typeof COLOR_TOKENS)[number];
export type Palette = Record<ColorToken, string>;

/** Spacing, §3.4: the only lengths a component may use. */
export const SPACE = { 1: 4, 2: 8, 3: 12, 4: 16, 5: 24, 6: 32, 7: 48, 8: 64 } as const;
export const RADIUS = { 1: 4, 2: 6, 3: 10 } as const;

/** Font stacks for canvas text, matching `--ff-body` and `--ff-mono`. */
export const FONT_VARS = { body: '--ff-body', mono: '--ff-mono' } as const;

function read(style: CSSStyleDeclaration, name: string): string {
  return style.getPropertyValue(`--${name}`).trim();
}

/** Resolves every colour token on `root` as it is styled right now. */
export function readPalette(root: Element = document.documentElement): Palette {
  const style = getComputedStyle(root);
  const out = {} as Palette;
  for (const token of COLOR_TOKENS) out[token] = read(style, token);
  return out;
}

/** Resolves a length token such as `row` or `spark-w` to pixels. */
export function readPx(name: string, root: Element = document.documentElement): number {
  const value = Number.parseFloat(read(getComputedStyle(root), name));
  return Number.isFinite(value) ? value : 0;
}

/**
 * Calls `onChange` with a fresh palette now and whenever the theme changes: the system
 * scheme, `data-theme`, or forced colours. Returns the unsubscribe function.
 */
export function watchPalette(onChange: (palette: Palette) => void): () => void {
  const root = document.documentElement;
  const emit = (): void => onChange(readPalette(root));
  const queries = ['(prefers-color-scheme: dark)', '(forced-colors: active)'].map((q) =>
    window.matchMedia(q),
  );
  for (const query of queries) query.addEventListener('change', emit);
  const observer = new MutationObserver(emit);
  observer.observe(root, { attributes: true, attributeFilter: ['data-theme'] });
  emit();
  return () => {
    for (const query of queries) query.removeEventListener('change', emit);
    observer.disconnect();
  };
}
