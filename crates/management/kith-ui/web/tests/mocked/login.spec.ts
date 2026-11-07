import { copy } from '../../src/lib/copy';
import { copyConsole } from '../../src/lib/copy.console';
import { ana, expect, isBase64url, network, office, test } from './console';

test('signed out, the page shows one button and nothing about the network', async ({ page, server, authenticator }) => {
  await authenticator.withKey();
  await page.goto(`/n/${network.name}`);
  const button = page.getByRole('button', { name: copyConsole.login.signIn });
  await expect(button).toBeVisible();

  const text = await page.locator('body').innerText();
  for (const secret of [network.name, ...(office.peers[network.id] ?? []).map((p) => p.name)]) {
    expect(text).not.toContain(secret);
  }
  expect(await page.title()).toBe(copy.brand);

  await button.click();
  await expect(page.getByRole('button', { name: copy.account.signOut })).toBeVisible();
  await expect(page.getByText(ana.name, { exact: true })).toBeVisible();
  await expect(page.getByText(copy.account.role.owner, { exact: true })).toBeVisible();
  const finish = server.sent('/api/login/finish')[0]?.body as { assertion: { id: string; response: Record<string, unknown> } };
  expect(isBase64url(finish.assertion.response['authenticatorData'])).toBe(true);
  expect(isBase64url(finish.assertion.response['userHandle'])).toBe(true);
});

test('a passkey kith does not know is explained', async ({ page, server, authenticator }) => {
  await authenticator.withKey();
  server.failOnce('/api/login/finish', 401, 'unauthorized');
  await page.goto('/');
  await page.getByRole('button', { name: copyConsole.login.signIn }).click();
  await expect(page.getByRole('alert')).toHaveText(copyConsole.login.unknown);
});

test('signs out back to the sign-in page', async ({ page, server }) => {
  server.signedIn = ana;
  await page.goto('/');
  await page.getByRole('button', { name: copy.account.signOut }).click();
  await expect(page.getByRole('button', { name: copyConsole.login.signIn })).toBeVisible();
  expect(server.sent('/api/logout')).toHaveLength(1);
});
