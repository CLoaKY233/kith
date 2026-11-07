// A device's "Internet traffic" controls: pick an exit and lock it, go back to the device's
// own connection (the combobox once dropped that empty choice), then pause it. The fixture
// server only replays, so the settings endpoint and the peer list answer as the daemon would.

import { copy } from '../../src/lib/copy';
import type { DeviceSettingsInfo, PeerInfo } from '../../src/lib/types.gen';
import { accept, devices, exits, expect, network, open, test } from './kith';

const target = devices.find((d) => d.name === 'work-laptop');
const vps = exits.find((e) => e.name === 'vps');

test('choose an exit and lock it, go back to its own connection, then pause', async ({ app }) => {
  if (!target || !vps) throw new Error('the office fixture needs work-laptop and vps');
  let current: DeviceSettingsInfo | null = null;
  const posted: DeviceSettingsInfo[] = [];
  await app.route('**/api/networks/*/devices/*/settings', async (route) => {
    current = route.request().postDataJSON() as DeviceSettingsInfo;
    posted.push(current);
    await accept(route);
  });
  await app.route('**/api/networks/*/peers', async (route) => {
    const peers: PeerInfo[] = devices.map((d) =>
      d.node === target.node ? { ...d, settings: current } : d,
    );
    await route.fulfill({ json: peers });
  });
  await open(app, `/n/${encodeURIComponent(network.name)}/devices/${encodeURIComponent(target.name)}`);

  const save = app.getByRole('button', { name: copy.deviceSettings.save, exact: true });
  const lock = app.getByRole('switch', { name: copy.deviceSettings.lock });
  const pause = app.getByRole('switch', { name: copy.deviceSettings.pause });
  const goesThrough = app.getByRole('button', { name: copy.deviceSettings.exit });

  await goesThrough.click();
  await app.getByRole('option', { name: vps.name, exact: true }).click();
  await lock.click();
  await save.click();
  await expect.poll(() => posted.length).toBe(1);
  expect(posted[0]).toEqual({ exit: vps.node, exit_locked: true, paused: false });
  await expect(save).toBeHidden();

  await goesThrough.click();
  await app.getByRole('option', { name: copy.deviceSettings.own }).click();
  await lock.click();
  await save.click();
  await expect.poll(() => posted.length).toBe(2);
  expect(posted[1]).toEqual({ exit: null, exit_locked: false, paused: false });
  await expect(save).toBeHidden();

  await pause.click();
  await save.click();
  await expect.poll(() => posted.length).toBe(3);
  expect(posted[2]).toEqual({ exit: null, exit_locked: false, paused: true });

  // Navigate in the page: a reload would render from the boot snapshot, not the saved state.
  await app.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: /^Devices/ }).click();
  const row = app.getByRole('row', { name: new RegExp(`^${target.name}$`) });
  await expect(row.getByText(copy.deviceSettings.tagPaused)).toBeVisible();
});
