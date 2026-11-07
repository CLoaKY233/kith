import { copy } from '../../src/lib/copy';
import { copyConsole } from '../../src/lib/copy.console';
import { expect, isBase64url, network, test, WORDS } from './console';

test('sets up the network in three steps with a passkey', async ({ page, server, authenticator }) => {
  void authenticator;
  await page.goto('/setup/tok-123');
  await expect(page.getByText(copyConsole.setup.progress(1, 3))).toBeVisible();

  const name = page.getByLabel(copyConsole.setup.networkLabel);
  await name.fill('Office Net');
  await expect(name).toHaveValue('office-net');
  await page.getByRole('button', { name: copyConsole.setup.continue }).click();
  await expect(page.getByText(copyConsole.setup.personEmpty)).toBeVisible();
  await page.getByLabel(copyConsole.setup.personLabel).fill('Ana');
  await page.getByRole('button', { name: copyConsole.setup.continue }).click();

  await expect(page.getByText(copyConsole.setup.progress(2, 3))).toBeVisible();
  expect(server.sent('/api/setup/begin')[0]?.body).toEqual({
    token: 'tok-123',
    name: 'Ana',
    network_name: 'office-net',
  });
  await page.getByRole('button', { name: copyConsole.setup.createPasskey }).click();
  await page.getByRole('button', { name: copyConsole.setup.confirm }).click();

  await expect(page.getByText(copyConsole.setup.progress(3, 3))).toBeVisible();
  const words = page.getByRole('list', { name: copyConsole.setup.wordsLabel }).getByRole('listitem');
  await expect(words).toHaveText(WORDS);

  const finish = server.sent('/api/setup/finish')[0]?.body as { token: string; credential: Record<string, unknown> };
  const response = finish.credential['response'] as Record<string, unknown>;
  expect(finish.token).toBe('tok-123');
  expect(isBase64url(finish.credential['rawId'])).toBe(true);
  expect(isBase64url(response['attestationObject'])).toBe(true);
  expect(isBase64url(response['clientDataJSON'])).toBe(true);
  const signed = server.sent('/api/authority/finish')[0]?.body as { credential: { response: Record<string, unknown> } };
  expect(isBase64url(signed.credential.response['signature'])).toBe(true);
  expect(server.sent('/api/setup/recovery')).toHaveLength(1);

  await page.getByRole('button', { name: copyConsole.setup.saved }).click();
  await page.waitForURL(`**/n/${network.name}`);
  await expect(page.getByRole('button', { name: copy.account.signOut })).toBeVisible();
});

test('explains a closed passkey prompt and lets the person try again', async ({ page, authenticator }) => {
  void authenticator;
  await page.goto('/setup/tok-123');
  await page.getByLabel(copyConsole.setup.personLabel).fill('Ana');
  await page.getByRole('button', { name: copyConsole.setup.continue }).click();

  await page.evaluate(() => {
    const create = navigator.credentials.create.bind(navigator.credentials);
    navigator.credentials.create = () => {
      navigator.credentials.create = create;
      return Promise.reject(new DOMException('The operation either timed out or was not allowed.', 'NotAllowedError'));
    };
  });
  await page.getByRole('button', { name: copyConsole.setup.createPasskey }).click();
  await expect(page.getByRole('alert')).toHaveText(copyConsole.passkey.cancelled);

  await page.getByRole('button', { name: copyConsole.setup.createPasskey }).click();
  await expect(page.getByRole('button', { name: copyConsole.setup.confirm })).toBeVisible();
});

test('says when the setup link expired', async ({ page, server }) => {
  server.failOnce('/api/setup/begin', 410, 'session_expired');
  await page.goto('/setup/old');
  await page.getByLabel(copyConsole.setup.personLabel).fill('Ana');
  await page.getByRole('button', { name: copyConsole.setup.continue }).click();
  await expect(page.getByRole('alert')).toHaveText(copyConsole.setup.expired);
  await expect(page.getByRole('button', { name: copyConsole.setup.continue })).toHaveCount(0);
});

test('says when this browser has no passkeys', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'PublicKeyCredential', { value: undefined });
  });
  await page.goto('/setup/tok-123');
  await expect(page.getByRole('alert')).toHaveText(copyConsole.passkey.unsupported);
});
