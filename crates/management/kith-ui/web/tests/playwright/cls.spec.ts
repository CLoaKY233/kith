// Cumulative layout shift is 0 after first paint (spec §11.9): numbers change in place,
// toasts and badges never move the layout, and nothing waits on a skeleton. Each screen is
// watched for two ticks, so live numbers commit while it is measured.

import { TICK_MS } from '../../src/lib/live/timing';
import { expect, network, open, test } from './kith';

const net = `/n/${encodeURIComponent(network.name)}`;
const SCREENS = [
  net,
  `${net}/devices`,
  `${net}/devices?view=map`,
  `${net}/activity`,
  `${net}/names`,
  `${net}/exits`,
  `${net}/settings`,
  '/this',
];
const WIDTHS = [360, 1280] as const;

declare global {
  interface Window {
    layoutShift: number;
  }
}

for (const width of WIDTHS) {
  test.describe(`${width} px`, () => {
    test.use({ viewport: { width, height: 800 } });

    for (const path of SCREENS) {
      test(path, async ({ app }) => {
        await app.addInitScript(() => {
          window.layoutShift = 0;
          new PerformanceObserver((list) => {
            for (const entry of list.getEntries() as (PerformanceEntry & {
              value: number;
              hadRecentInput: boolean;
            })[]) {
              if (!entry.hadRecentInput) window.layoutShift += entry.value;
            }
          }).observe({ type: 'layout-shift', buffered: true });
        });
        await open(app, path);
        await app.waitForTimeout(2 * TICK_MS + TICK_MS / 2);
        expect(await app.evaluate(() => window.layoutShift)).toBe(0);
      });
    }
  });
}
