// Colours for canvas drawing. Canvas pixels are not adjusted by forced-colours mode, so
// under it every stroke and label uses the chart ink (CanvasText, §3.2) instead.

import type { ColorToken, Palette } from '../tokens';

export function ink(palette: Palette, token: ColorToken): string {
  const forced =
    typeof window !== 'undefined' && window.matchMedia('(forced-colors: active)').matches;
  return forced ? palette['chart-ink'] : palette[token];
}
