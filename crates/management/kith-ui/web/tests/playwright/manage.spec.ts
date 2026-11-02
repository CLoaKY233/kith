// Access, relays, invites, server mode and WireGuard import against the office fixture.
// The fixture server only replays, so each spec answers the newer endpoints itself, as the
// daemon would, and checks what the page sent.

import { copy } from '../../src/lib/copy';
import { policyToml } from '../../src/lib/kith/policy';
import type { PolicyRule, Relay } from '../../src/lib/types';
import { accept, devices, expect, network, open, test } from './kith';

const net = `/n/${encodeURIComponent(network.name)}`;
const nas = devices.find((d) => d.name !== '') ?? devices[0];
const rule: PolicyRule = {
  sources: [{ kind: 'tag', tag: 'phones' }],
  destinations: [
    { selector: { kind: 'device', node: nas?.node ?? '' }, protocol: 'tcp', ports: [{ start: 22, end: 22 }] },
  ],
};

test('Access reads the policy as sentences and saves a new rule', async ({ app }) => {
  const saved: string[] = [];
  await app.route('**/api/networks/*/policy', async (route) => {
    if (route.request().method() === 'POST') {
      saved.push((route.request().postDataJSON() as { toml: string }).toml);
      await route.fulfill({ json: {} });
    } else {
      await route.fulfill({ json: { toml: policyToml([rule]), rules: [rule] } });
    }
  });
  await open(app, `${net}/access`);
  await expect(app.getByText(`Phones can reach ${nas?.name ?? ''} on TCP port 22.`)).toBeVisible();

  await app.getByLabel(copy.access.ports).fill('443');
  await app.getByRole('button', { name: copy.access.addRule }).click();
  await expect(app.getByText(copy.access.unsaved(1))).toBeVisible();
  await app.getByRole('button', { name: copy.access.save }).click();
  await expect(app.getByText(copy.access.saved)).toBeVisible();
  expect(saved).toHaveLength(1);
  expect(saved[0]).toContain('ports = { Only = [{ start = 443, end = 443 }] }');
});

test('a TOML parse error shows at its line and column', async ({ app }) => {
  await app.route('**/api/networks/*/policy', async (route) => {
    if (route.request().method() === 'POST') {
      await route.fulfill({
        status: 400,
        json: { code: 'invalid_argument', message: 'policy: TOML parse error at line 1, column 3\ninvalid key' },
      });
    } else {
      await route.fulfill({ json: { toml: 'rules = []\n', rules: [] } });
    }
  });
  await open(app, `${net}/access`);
  await app.getByRole('tab', { name: copy.access.views.toml }).click();
  await app.getByLabel(copy.access.toml).fill('ru les');
  await app.getByRole('button', { name: copy.access.save }).click();
  // The live region announces it too; the visible line is in the TOML tab.
  await expect(
    app.getByRole('tabpanel').getByText(copy.access.tomlError(1, 3, 'invalid key')),
  ).toBeVisible();
});

test('Settings lists relays and open invites, and changes them', async ({ app }) => {
  let relays: Relay[] = [{ name: 'main', address: 'vps.example.com:41642', node: null }];
  const posted: string[] = [];
  await app.route('**/api/networks/*/relays**', async (route) => {
    const request = route.request();
    if (request.method() === 'GET') return route.fulfill({ json: relays });
    posted.push(new URL(request.url()).pathname);
    if (request.url().endsWith('/remove')) relays = [];
    else relays = [...relays, request.postDataJSON() as Relay];
    await accept(route);
  });
  await app.route('**/api/networks/*/invites', (route) =>
    route.fulfill({ json: [{ id: 'i1', expires_at_ms: Date.now() + 3_600_000, uses_left: 1, created_at_ms: Date.now() }] }),
  );
  await app.route('**/api/networks/*/invites/*/revoke', accept);
  await open(app, `${net}/settings`);

  await expect(app.getByText('main', { exact: true })).toBeVisible();
  await app.getByLabel(copy.relays.name).fill('backup');
  await app.getByLabel(copy.relays.address).fill('relay.example.com:443');
  await app.getByRole('button', { name: copy.relays.add }).click();
  await expect(app.getByText('backup', { exact: true })).toBeVisible();
  await app.getByRole('button', { name: copy.relays.removeLabel('main') }).click();
  await expect(app.getByText(copy.relays.removed('main'))).toBeVisible();
  expect(posted.at(-1)).toMatch(/\/relays\/main\/remove$/);

  await expect(app.getByText(/1 use left/)).toBeVisible();
});

test('offering this device as a relay asks first, then saves', async ({ app }) => {
  const asked: unknown[] = [];
  await app.route('**/api/server', async (route) => {
    if (route.request().method() === 'POST') {
      asked.push(route.request().postDataJSON());
      return route.fulfill({ json: {} });
    }
    await route.fulfill({ json: { exit: false, relay: false } });
  });
  await open(app, '/this');
  await app.getByRole('switch', { name: copy.server.relay }).click();
  await app.getByRole('alertdialog').getByRole('button', { name: /^Offer .* as a relay$/ }).click();
  await expect.poll(() => asked).toEqual([{ exit: false, relay: true }]);
});

test('a WireGuard config imports into Exits, where it is used, stopped and removed', async ({ app }) => {
  const conf =
    '[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nAddress = 10.64.0.2/32\n\n' +
    '[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\n' +
    'Endpoint = 198.51.100.1:51820\nAllowedIPs = 0.0.0.0/0, ::/0\n';
  await open(app, `${net}/exits`);
  const imports = app.getByRole('list', { name: copy.wgImport.heading });
  await expect(imports.getByText('work-vpn', { exact: true })).toBeVisible();

  await app.getByRole('button', { name: copy.wgImport.open }).click();
  await app.getByLabel(copy.wgImport.name).fill('mullvad');
  await app.getByLabel(copy.wgImport.conf, { exact: true }).fill(conf);
  await app.getByRole('button', { name: copy.wgImport.submit }).click();
  await expect(
    app.getByRole('region', { name: copy.layer.toasts }).getByText(copy.wgImport.done('mullvad')),
  ).toBeVisible();
  const row = imports.getByRole('listitem').filter({ hasText: 'mullvad' });
  await expect(row.getByText(copy.wgImport.everything)).toBeVisible();
  await expect(row.getByText(copy.wgImport.inUse, { exact: true })).toBeVisible();

  await row.getByRole('button', { name: copy.wgImport.stop('mullvad') }).click();
  await expect(row.getByText(copy.wgImport.notInUse)).toBeVisible();
  await row.getByRole('button', { name: copy.wgImport.use('mullvad') }).click();
  await expect(row.getByText(copy.wgImport.inUse, { exact: true })).toBeVisible();

  await row.getByRole('button', { name: copy.wgImport.moreFor('mullvad') }).click();
  await app.getByRole('menuitem', { name: copy.wgImport.remove }).click();
  await app.getByRole('button', { name: copy.wgImport.removeSubmit('mullvad') }).click();
  await expect(row).toHaveCount(0);
  await expect(imports.getByText('work-vpn', { exact: true })).toBeVisible();
});
