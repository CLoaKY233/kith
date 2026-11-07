// Colours for canvas drawing. Canvas pixels are not adjusted by forced-colours mode, so
// under it every stroke and label uses the chart ink (CanvasText, §3.2) instead.

import type { ColorToken, Palette } from '../tokens';

export function ink(palette: Palette, token: ColorToken): string {
  const forced =
    typeof window !== 'undefined' && window.matchMedia('(forced-colors: active)').matches;
  return forced ? palette['chart-ink'] : palette[token];
}

/**
 * The area under a traffic line: `color` at `alpha` (two hex digits) at y = `from`, fading
 * to nothing at the baseline, y = `to`. The only gradient on the page. Tokens are
 * `#rrggbb`; a forced colour (a keyword) gets no area, the line alone carries the data.
 */
export function wash(
  ctx: CanvasRenderingContext2D,
  color: string,
  alpha: string,
  from: number,
  to: number,
): CanvasGradient | string {
  if (color.length !== 7) return 'transparent';
  const g = ctx.createLinearGradient(0, from, 0, to);
  g.addColorStop(0, color + alpha);
  g.addColorStop(1, `${color}00`);
  return g;
}
