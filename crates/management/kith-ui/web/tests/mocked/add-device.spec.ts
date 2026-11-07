import { copy } from '../../src/lib/copy';
import { copyConsole } from '../../src/lib/copy.console';
import { ana, expect, network, push, test } from './console';

test('a computer joins with a code and the page says whose it is', async ({ page, server }) => {
  server.signedIn = ana;
  await page.goto(`/n/${network.name}?add=device`);
  const sheet = page.getByRole('dialog', { name: copy.addDevice.title });
  await sheet.getByRole('button', { name: copy.addDevice.laptop }).click();

  await expect(sheet.getByText('K7Q·4XM')).toBeVisible();
  await expect(sheet.getByText('kith join https://vpn.example.com/j/K7Q4XM')).toBeVisible();
  expect(server.sent('/api/device-codes')[0]?.body).toEqual({ person: ana.id });
  await expect(sheet.getByText(copyConsole.deviceCode.waiting)).toBeVisible();

  await push(page, {
    kind: 'device_joined',
    network: network.id,
    node: 'n-dev-box',
    name: 'dev-box',
    address: '100.64.0.40',
  });
  await expect(sheet.getByText(copyConsole.deviceCode.joined('dev-box', ana.name))).toBeVisible();
});

test('a phone still uses the QR flow', async ({ page, server }) => {
  server.signedIn = ana;
  await page.goto(`/n/${network.name}?add=device`);
  await page.getByRole('dialog', { name: copy.addDevice.title }).getByRole('button', { name: copy.addDevice.phone }).click();
  await expect(page).toHaveURL(/add=phone/);
  expect(server.sent('/api/device-codes')).toHaveLength(0);
});
