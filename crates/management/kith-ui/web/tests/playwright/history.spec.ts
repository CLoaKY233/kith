// What the server keeps past the live hour (GET /api/history/*), against the office
// fixture: traffic per day on a device, and Activity reading back past what the page holds.

import { copy } from '../../src/lib/copy';
import { copyConsole } from '../../src/lib/copy.console';
import type { TrafficHistory } from '../../src/lib/types.gen';
import { devices, expect, network, open, test } from './kith';

const net = `/n/${encodeURIComponent(network.name)}`;
const busy = devices.find((d) => d.online) ?? devices[0];

test('a device shows its traffic by day from what the server kept', async ({ app }) => {
  const node = busy?.node ?? '';
  // The server rolls the 2 s ticks up into minutes as they come.
  await expect
    .poll(async () => {
      const kept = (await (
        await app.request.get(`/api/history/traffic?network=${network.id}&node=${node}`)
      ).json()) as TrafficHistory[];
      return kept.length;
    })
    .toBeGreaterThan(0);
  await open(app, `${net}/devices/${encodeURIComponent(busy?.name ?? '')}`);
  await app.getByRole('button', { name: copyConsole.history.daysShow }).click();
  const table = app.getByRole('table', { name: copyConsole.history.daysCaption });
  await expect(table.getByRole('rowheader', { name: copy.time.today })).toBeVisible();
});

test('Activity reads back past what the page holds', async ({ app }) => {
  await open(app, `${net}/activity`);
  // The control is the feed's last row, so it renders once the feed is scrolled to its end.
  const older = app.getByRole('button', { name: copyConsole.history.older });
  await expect
    .poll(async () => {
      await app.evaluate(() => {
        for (const element of [document.scrollingElement, ...document.querySelectorAll('*')]) {
          if (element && element.scrollHeight > element.clientHeight) element.scrollTop = element.scrollHeight;
        }
      });
      return older.count();
    })
    .toBe(1);
  const asked = app.waitForRequest((request) => request.url().includes('/api/history/journal'));
  await older.click();
  await asked;
  await expect(app.getByText(copyConsole.history.olderNone)).toBeVisible();
});
