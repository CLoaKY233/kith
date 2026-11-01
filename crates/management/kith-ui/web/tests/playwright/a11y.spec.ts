// axe-core at 360, 768, 1280 and 1920 px in both themes, with zero violations, on every
// screen and overlay (spec §11.6). The same pass checks that nothing scrolls sideways
// (§4.4) at any width.

import AxeBuilder from '@axe-core/playwright';
import type { Page } from '@playwright/test';
import { devices, expect, joinRequest, network, open, settle, test, thisDevice } from './kith';

const WIDTHS = [
  { width: 360, height: 780, touch: true },
  { width: 768, height: 1024, touch: false },
  { width: 1280, height: 800, touch: false },
  { width: 1920, height: 1080, touch: false },
] as const;
const THEMES = ['light', 'dark'] as const;
const TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'];

const net = `/n/${encodeURIComponent(network.name)}`;
const other = devices.find((device) => device.name !== thisDevice)?.name ?? thisDevice;
const dev = `${net}/devices/${encodeURIComponent(other)}`;

/** Every route of §4.1 and every overlay its query parameters open. */
const SCREENS: readonly (readonly [string, string])[] = [
  ['Welcome', '/welcome'],
  ['This device', '/this'],
  ['Overview', net],
  ['Devices', `${net}/devices`],
  ['Devices as a map', `${net}/devices?view=map`],
  ['Device', dev],
  ['Activity', `${net}/activity`],
  ['Activity logs', `${net}/activity?kind=logs`],
  ['Names', `${net}/names`],
  ['Exits', `${net}/exits`],
  ['Settings', `${net}/settings`],
  ['Add device', `${net}?add=device`],
  ['Add a phone', `${net}?add=phone`],
  ['Invite', `${net}?add=invite`],
  ['Join request', `${net}?need=${encodeURIComponent(joinRequest.id)}`],
  ['Remove device', `${dev}?remove=${encodeURIComponent(other)}`],
];

async function audit(page: Page): Promise<void> {
  const results = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  const violations = results.violations.map(
    (violation) =>
      `${violation.id}: ${violation.help} at ${violation.nodes
        .map((node) => node.target.join(' '))
        .join(', ')}`,
  );
  expect(violations).toEqual([]);
  const sideways = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(sideways, 'nothing scrolls sideways').toBeLessThanOrEqual(0);
}

for (const { width, height, touch } of WIDTHS) {
  for (const theme of THEMES) {
    test.describe(`${width} px, ${theme}`, () => {
      test.use({
        viewport: { width, height },
        colorScheme: theme,
        hasTouch: touch,
        isMobile: touch,
      });

      for (const [name, path] of SCREENS) {
        test(name, async ({ app }) => {
          await open(app, path);
          await audit(app);
        });
      }

      test('Gate for a fresh link', async ({ page }) => {
        await page.goto('/');
        await settle(page);
        await audit(page);
      });
    });
  }
}
