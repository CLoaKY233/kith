import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test, expect, type Browser, type BrowserContext, type Page } from '@playwright/test';

test.describe.configure({ mode: 'serial' });

const setupUrl = () => readFileSync(join(import.meta.dirname, '.setup-url'), 'utf8').trim();

async function withPasskey(ctx: BrowserContext, page: Page) {
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
  });
}

function watchErrors(page: Page) {
  const errors: string[] = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(`${page.url()} console: ${m.text()}`); });
  page.on('pageerror', (e) => errors.push(`${page.url()} pageerror: ${e}`));
  return errors;
}

async function newPasskeyPage(browser: Browser) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  await withPasskey(ctx, page);
  return { ctx, page, errors: watchErrors(page) };
}

async function auditPage(page: Page) {
  return page.evaluate(() => {
    const problems: string[] = [];
    const vw = document.documentElement.clientWidth;
    if (document.documentElement.scrollWidth > vw + 1) problems.push(`horizontal overflow ${document.documentElement.scrollWidth} > ${vw}`);
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const t = n.textContent ?? '';
      const el = n.parentElement;
      if (!el || el.closest('script,style,svg') || !el.getClientRects().length) continue;
      if (/\b[0-9a-f]{20,}\b/.test(t)) problems.push(`raw hex id: ${t.trim().slice(0, 60)}`);
      if (/\b(undefined|NaN)\b/.test(t)) problems.push(`raw js value: ${t.trim().slice(0, 60)}`);
    }
    return problems;
  });
}

let browser: Browser;
let ctx: BrowserContext;
let page: Page;
let errors: string[];

test.beforeAll(async ({ browser: b }) => {
  browser = b;
  ({ ctx, page, errors } = await newPasskeyPage(browser));
});
test.afterAll(async () => { await ctx.close(); });

test('setup creates the owner and lands on Overview signed in', async () => {
  await page.goto(setupUrl());
  const inputs = page.locator('input:visible');
  await inputs.first().waitFor();
  const count = await inputs.count();
  for (let i = 0; i < count; i++) {
    const el = inputs.nth(i);
    if (!(await el.inputValue())) await el.fill(i === 0 ? 'Lay Sheth' : 'home');
  }
  await page.getByRole('button', { name: /continue|next/i }).first().click();
  await page.getByRole('button', { name: /create|passkey/i }).first().click();
  await page.getByRole('button', { name: /confirm|sign|continue/i }).first().click();
  await page.getByRole('button', { name: /saved/i }).first().click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(/overview/i);
  await expect(page.getByRole('button', { name: /^Account/ })).toBeVisible();
});

const ROUTES = ['/', '/devices', '/devices?view=map', '/devices/vps', '/stats', '/activity',
  '/settings/network', '/settings/exits', '/settings/names', '/settings/access', '/settings/people'];

for (const route of ROUTES) {
  test(`${route} renders cleanly at 390 and 1440`, async () => {
    for (const width of [390, 1440]) {
      errors.length = 0;
      await page.setViewportSize({ width, height: width < 768 ? 844 : 900 });
      await page.goto(route);
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await page.waitForLoadState('networkidle');
      expect(await auditPage(page), `at ${width}`).toEqual([]);
      expect(errors, `at ${width}`).toEqual([]);
    }
    await page.setViewportSize({ width: 1440, height: 900 });
  });
}

test('account menu shows the name; sign out and back in with the passkey', async () => {
  await page.goto('/');
  await page.getByRole('button', { name: /^Account/ }).click();
  await expect(page.getByRole('menu')).toContainText('Lay Sheth');
  await page.getByRole('menuitem', { name: /sign out/i }).click();
  await expect(page).toHaveURL(/\/login/);
  await page.getByRole('button', { name: /sign in/i }).first().click();
  await expect(page).not.toHaveURL(/\/login/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(/overview/i);
});

test('People: add, register in another browser, approve with passkey', async () => {
  await page.goto('/settings/people');
  await page.getByRole('button', { name: /add person/i }).first().click();
  const dialog = page.getByRole('dialog');
  await dialog.locator('input').first().fill('Alex Morgan');
  await dialog.getByRole('button', { name: /create|add|invite|continue/i }).last().click();
  await expect(dialog).toContainText(/\/register\//);
  const text = (await dialog.innerText()) + ' ' + (await dialog.locator('input').evaluateAll((els) => els.map((e) => (e as HTMLInputElement).value).join(' ')));
  const link = text.match(/https?:\/\/\S+\/register\/[0-9A-Za-z_-]+/)?.[0];
  expect(link, 'register link in dialog').toBeTruthy();
  await page.keyboard.press('Escape');

  const other = await newPasskeyPage(browser);
  try {
    await other.page.goto(new URL(link!).pathname);
    await other.page.getByRole('button', { name: /create|passkey/i }).first().click();
    await expect(other.page.getByText(/approv|waiting|done|registered/i).first()).toBeVisible();
    expect(other.errors).toEqual([]);
  } finally { await other.ctx.close(); }

  await page.goto('/settings/people');
  const row = page.getByText('Alex Morgan').first();
  await expect(row).toBeVisible();
  await page.getByRole('button', { name: /approve/i }).first().click();
  await page.getByRole('alertdialog').getByRole('button', { name: /approve|confirm/i }).last().click();
  await expect(page.getByRole('alertdialog')).toBeHidden();
  await expect(page.getByRole('button', { name: /approve/i })).toHaveCount(0);
});

test.describe('signed out', () => {
  for (const [name, path] of [['login', '/login'], ['recover', '/recover'], ['bad register link', '/register/bad'], ['used setup link', '']] as const) {
    test(`${name} renders a sensible page`, async ({ page: p }) => {
      const errs = watchErrors(p);
      await p.goto(path || new URL(setupUrl()).pathname);
      await expect(p.getByRole('heading', { level: 1 })).toBeVisible();
      await p.waitForLoadState('networkidle');
      const body = await p.locator('body').innerText();
      expect(body.trim().length).toBeGreaterThan(20);
      if (path === '' || path === '/register/bad') expect(body).toMatch(/invalid|expired|used|no longer|not valid|sign in/i);
      expect(await auditPage(p)).toEqual([]);
      expect(errs.filter((e) => !/Failed to load resource: .*status of 4(01|03|04|10)/.test(e))).toEqual([]);
    });
  }
});
