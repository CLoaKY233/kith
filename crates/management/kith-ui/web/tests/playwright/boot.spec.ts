// Signing in, the guards around it, and the live stream (spec §2, §8.1, §8.5).

import { copy } from '../../src/lib/copy';
import { TICK_MS } from '../../src/lib/live/timing';
import { expect, network, open, test, thisDevice } from './kith';

const CSP =
  "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

test('the one-time link signs in, drops its token and lands on the first network', async ({
  app,
  context,
  server,
}) => {
  expect(new URL(app.url()).hash).toBe('');
  await expect(app).toHaveTitle(copy.documentTitle(thisDevice, network.name, 0));
  const cookies = await context.cookies();
  expect(cookies).toEqual([
    // One cookie per port, so two kith-ui on one machine keep their own sessions.
    expect.objectContaining({
      name: `kith_ui_${new URL(server.origin).port}`,
      httpOnly: true,
      sameSite: 'Strict',
      path: '/',
    }),
  ]);
});

test('a signed-in load renders from the inline snapshot without fetching it', async ({ app }) => {
  const fetched: string[] = [];
  app.on('request', (request) => {
    if (new URL(request.url()).pathname === '/api/boot') fetched.push(request.url());
  });
  await open(app, `/n/${encodeURIComponent(network.name)}/devices`);
  const boot = JSON.parse((await app.locator('script#boot').textContent()) ?? 'null') as {
    boot?: string;
    seq?: number;
  } | null;
  expect(boot?.boot).toBeTruthy();
  expect(typeof boot?.seq).toBe('number');
  expect(fetched).toEqual([]);
});

test('every answer carries the content security policy', async ({ app }) => {
  const page = await app.request.get(`/n/${encodeURIComponent(network.name)}/devices`);
  expect(page.headers()['content-security-policy']).toBe(CSP);
  const api = await app.request.get('/api/status');
  expect(api.headers()['content-security-policy']).toBe(CSP);
});

test('a used link shows the gate, which says links work once', async ({ app, browser, server }) => {
  expect(app.url()).not.toContain('#t=');
  const second = await browser.newPage();
  await second.goto(server.link);
  await expect(second.getByRole('heading', { name: copy.gate.authTitle })).toBeVisible();
  await expect(second.getByText(copy.gate.authUsed)).toBeVisible();
  await expect(second.getByText(copy.gate.authCommand, { exact: true })).toBeVisible();
  await second.close();
});

test('without a session the page explains itself with no JavaScript', async ({
  browser,
  server,
}) => {
  const context = await browser.newContext({ javaScriptEnabled: false, baseURL: server.origin });
  const page = await context.newPage();
  await page.goto('/');
  await expect(page.getByRole('heading', { name: copy.gate.authTitle })).toBeVisible();
  await expect(page.getByText(copy.gate.authBody)).toBeVisible();
  await expect(page.getByText(copy.gate.authOnce)).toBeVisible();
  await context.close();
});

test('only the spec routes exist', async ({ app }) => {
  const net = `/n/${encodeURIComponent(network.name)}`;
  for (const path of [`${net}/inbox`, `${net}/devices/nas/x`, '/admin', '/index.html']) {
    expect((await app.request.get(path)).status(), path).toBe(404);
  }
});

test('the stream sends a tick every 2 seconds to this page', async ({ app }) => {
  const gap = await app.evaluate(
    (timeout) =>
      new Promise<number>((resolve, reject) => {
        const source = new EventSource('/api/events');
        const seen: number[] = [];
        const timer = setTimeout(() => {
          source.close();
          reject(new Error(`ticks seen: ${seen.length}`));
        }, timeout);
        source.addEventListener('tick', () => {
          seen.push(performance.now());
          const [a, b] = seen;
          if (a === undefined || b === undefined) return;
          clearTimeout(timer);
          source.close();
          resolve(b - a);
        });
      }),
    3 * TICK_MS,
  );
  expect(gap).toBeGreaterThan(TICK_MS / 2);
  expect(gap).toBeLessThan(TICK_MS * 1.5);
});

test('a live page shows no connection state', async ({ app }) => {
  await open(app, `/n/${encodeURIComponent(network.name)}/devices`);
  // Longer than stale (5 s) plus the banner delay (3 s): a live stream shows nothing.
  await app.waitForTimeout(5 * TICK_MS);
  await expect(app.getByText(copy.conn.staleActions)).toHaveCount(0);
  await expect(app.getByText(copy.conn.reconnecting)).toHaveCount(0);
  await expect(app.getByText(copy.header.staleAsOf('').trim())).toHaveCount(0);
});

test('when kith-ui stops, the page says so and how to reopen it', async ({ app, server }) => {
  await server.stop();
  await expect(app.getByText(copy.conn.uiGone)).toBeVisible({ timeout: 6 * TICK_MS });
});
