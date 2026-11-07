// The Console end to end against the real server: `kith-ui --fixture-console` on a fixture
// device in no network, people and passkeys in its in-memory ledger, which checks every
// signature as the real one does. Chrome's virtual authenticator stands in for Touch ID;
// each person has their own browser context and authenticator.

import { expect, test, type Browser, type BrowserContext, type Page } from '@playwright/test';
import { copy } from '../../src/lib/copy';
import { copyConsole } from '../../src/lib/copy.console';
import type { JoinLink } from '../../src/lib/types.gen';
import { startConsole, type ConsoleServer } from './server';

const NETWORK = 'office';

/** A browser context of its own, with a platform authenticator that always verifies. */
async function person(browser: Browser, server: ConsoleServer): Promise<{ context: BrowserContext; page: Page }> {
  const context = await browser.newContext({ baseURL: server.origin, locale: 'en-GB', timezoneId: 'UTC' });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error' && message.text().includes('Content Security Policy')) problems.push(message.text());
  });
  return { context, page };
}

/** Uncaught errors and CSP violations on any person's page; the test ends with none. */
let problems: string[] = [];

function row(page: Page, name: string) {
  return page
    .getByRole('list', { name: copyConsole.people.title })
    .getByRole('listitem')
    .filter({ has: page.getByText(name, { exact: true }) });
}

/** Invites `name` as `role` from the People page, and answers the link it shows. */
async function invite(page: Page, name: string, role: 'member' | 'admin'): Promise<string> {
  await page.getByRole('button', { name: copyConsole.people.add }).click();
  const sheet = page.getByRole('dialog', { name: copyConsole.people.add });
  await sheet.getByLabel(copyConsole.people.nameLabel).fill(name);
  if (role === 'admin') await sheet.getByRole('radio', { name: copy.account.role.admin }).click();
  await sheet.getByRole('button', { name: copyConsole.people.createLink }).click();
  if (role === 'admin') {
    // An admin's link works only once the owner's passkey confirmed it.
    await expect(sheet.getByText(copyConsole.people.confirmAdminLede(name))).toBeVisible();
    await sheet.getByRole('button', { name: copyConsole.people.confirmAdmin }).click();
  }
  await expect(sheet.getByText(copyConsole.people.linkFor(name))).toBeVisible();
  const link = await sheet.getByText(/^http:\/\/localhost:\d+\/register\/[0-9a-f]+$/).first().textContent();
  await sheet.getByRole('button', { name: copyConsole.people.done }).click();
  await expect(sheet).toBeHidden();
  return link ?? '';
}

/** The invited person opens their link in their own browser and makes a passkey. */
async function register(browser: Browser, server: ConsoleServer, link: string, inviter: string) {
  const them = await person(browser, server);
  await them.page.goto(link);
  await expect(them.page.getByRole('heading', { name: copyConsole.register.title(NETWORK) })).toBeVisible();
  await them.page.getByRole('button', { name: copyConsole.setup.createPasskey }).click();
  await expect(them.page.getByText(copyConsole.register.made(inviter))).toBeVisible();
  return them;
}

/** Approves someone who made their passkey: one button, then the owner's passkey. */
async function approve(page: Page, name: string): Promise<void> {
  await page.reload();
  const waiting = row(page, name);
  await expect(waiting).toHaveAttribute('data-state', 'registered');
  await waiting.getByRole('button', { name: copyConsole.people.approve }).click();
  const dialog = page.getByRole('alertdialog', { name: copyConsole.people.approveTitle(name) });
  await dialog.getByRole('button', { name: copyConsole.people.approveLabel(name) }).click();
  await expect(dialog).toBeHidden();
  await expect(row(page, name)).toHaveAttribute('data-state', 'active');
}

async function signIn(page: Page): Promise<void> {
  await page.goto('/login');
  await page.getByRole('button', { name: copyConsole.login.signIn }).click();
  await page.waitForURL(`**/n/${NETWORK}`);
}

let server: ConsoleServer;

test.beforeEach(async () => {
  problems = [];
  server = await startConsole();
});

test.afterEach(async () => {
  await server.stop();
});

test('sets the Console up, signs in and out, and manages people and devices', async ({ browser }) => {
  test.setTimeout(120_000);
  const owner = await person(browser, server);
  const page = owner.page;

  await test.step('setup: network, owner, passkey, authority, recovery words', async () => {
    await page.goto(server.setup);
    await page.getByLabel(copyConsole.setup.networkLabel).fill(NETWORK);
    await page.getByLabel(copyConsole.setup.personLabel).fill('Ana');
    await page.getByRole('button', { name: copyConsole.setup.continue }).click();
    await page.getByRole('button', { name: copyConsole.setup.createPasskey }).click();
    await expect(page.getByText(copyConsole.setup.confirmLede(NETWORK))).toBeVisible();
    await page.getByRole('button', { name: copyConsole.setup.confirm }).click();
    const words = page.getByRole('list', { name: copyConsole.setup.wordsLabel }).getByRole('listitem');
    await expect(words.first()).toBeVisible();
    expect(await words.count()).toBeGreaterThan(0);
    await page.getByRole('button', { name: copyConsole.setup.saved }).click();
    await page.waitForURL(`**/n/${NETWORK}`);
    await expect(page.getByText('Ana', { exact: true }).first()).toBeVisible();
    await expect(page.getByText(copy.account.role.owner, { exact: true }).first()).toBeVisible();
  });

  await test.step('the setup link works once', async () => {
    const again = await owner.context.request.get(`${server.setup.replace('/setup/', '/api/setup/')}`);
    expect(again.status()).toBe(410);
  });

  await test.step('signs out, then in with the passkey', async () => {
    await page.getByRole('button', { name: copy.account.signOut }).click();
    await page.waitForURL('**/login');
    await expect(page.getByRole('button', { name: copyConsole.login.signIn })).toBeVisible();
    expect((await page.request.get('/api/me')).status()).toBe(401);
    await page.getByRole('button', { name: copyConsole.login.signIn }).click();
    await page.waitForURL(`**/n/${NETWORK}`);
    await expect(page.getByRole('button', { name: copy.account.signOut })).toBeVisible();
  });

  await page.goto(`/n/${NETWORK}/people`);
  await expect(row(page, 'Ana')).toContainText(copyConsole.people.you);

  const bo = await test.step('adds a member by link, who registers and is approved', async () => {
    const link = await invite(page, 'Bo', 'member');
    await expect(row(page, 'Bo')).toContainText(copyConsole.people.pending);
    const them = await register(browser, server, link, 'Ana');
    await approve(page, 'Bo');
    await expect(row(page, 'Bo')).toContainText(copy.account.role.member);
    await signIn(them.page);
    await expect(them.page.getByText(copy.account.role.member, { exact: true }).first()).toBeVisible();
    return them;
  });

  await test.step('adds an admin, confirmed with the link and approved once registered', async () => {
    const link = await invite(page, 'Cy', 'admin');
    const them = await register(browser, server, link, 'Ana');
    await approve(page, 'Cy');
    await expect(row(page, 'Cy')).toContainText(copy.account.role.admin);
    await them.context.close();
  });

  await test.step('makes the member an admin', async () => {
    await page.getByRole('button', { name: copyConsole.people.actions('Bo') }).click();
    await page.getByRole('menuitem', { name: copyConsole.people.makeAdmin }).click();
    const dialog = page.getByRole('alertdialog', { name: copyConsole.people.makeAdminTitle('Bo') });
    await dialog.getByRole('button', { name: copyConsole.people.makeAdmin }).click();
    await expect(dialog).toBeHidden();
    await expect(row(page, 'Bo')).toContainText(copy.account.role.admin);
  });

  await test.step('removes a person, which ends their sessions', async () => {
    await page.getByRole('button', { name: copyConsole.people.actions('Bo') }).click();
    await page.getByRole('menuitem', { name: copyConsole.people.remove }).click();
    const dialog = page.getByRole('alertdialog', { name: copyConsole.people.removeTitle('Bo') });
    await dialog.getByRole('button', { name: copyConsole.people.removeConfirm('Bo') }).click();
    await expect(dialog).toBeHidden();
    await expect(row(page, 'Bo')).toHaveCount(0);
    expect((await bo.page.request.get('/api/me')).status()).toBe(401);
    await bo.context.close();
  });

  await test.step('makes a device code, and a computer fetches its invite once', async () => {
    await page.goto(`/n/${NETWORK}?add=device`);
    const sheet = page.getByRole('dialog', { name: copy.addDevice.title });
    await sheet.getByRole('button', { name: copy.addDevice.laptop }).click();
    await expect(sheet.getByText(copyConsole.deviceCode.forPerson('Ana'))).toBeVisible();
    const command = await sheet
      .getByText(/^kith join http:\/\/localhost:\d+\/j\/[0-9A-Z]{6}$/)
      .first()
      .textContent();
    const link = (command ?? '').replace('kith join ', '');
    const code = link.slice(-6);
    await expect(sheet.getByText(copy.spoken.checkCode(code))).toBeAttached();

    // `kith join <link>` asks with no cookie and no Origin; the code is all it needs, typed
    // in any case.
    const answer = await fetch(link.replace(code, code.toLowerCase()));
    expect(answer.status).toBe(200);
    const join = (await answer.json()) as JoinLink;
    expect(join.network_name).toBe(NETWORK);
    expect(join.invite_link).toMatch(/^kith:\/\/join\//);
    expect((await fetch(link)).status).toBe(410);
  });

  await owner.context.close();
  expect(problems, 'no uncaught errors or CSP violations').toEqual([]);
});
