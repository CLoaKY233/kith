// The test fixtures every spec uses: a fresh fixture server per test, a signed-in page that
// fails the test on uncaught errors or CSP violations, the office fixture's facts, and a
// key counter for the keyboard-only flows (§11.7).

import { test as base, expect, type Locator, type Page, type Route } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { DOT } from '../../src/lib/copy';
import type { Fixture, Outcome } from '../../src/lib/types.gen';
import { fixturePath, startServer, type Server } from './server';

function first<T>(items: readonly T[] | undefined, what: string): T {
  const item = items?.[0];
  if (item === undefined) throw new Error(`the office fixture has no ${what}`);
  return item;
}

/** The recorded network every test runs against. */
export const office = JSON.parse(readFileSync(fixturePath, 'utf8')) as Fixture;
export const network = first(office.status.networks, 'network');
export const thisDevice = first(office.status.memberships, 'membership').name;
export const joinRequest = first(office.pending.joins, 'join request');
export const blockedGroup = first(office.pending.blocked, 'blocked connection');
export const exits = office.exits[network.id] ?? [];
export const devices = office.peers[network.id] ?? [];

/** Every pointer action has a keyboard path of fewer than this many keys (§11.7). */
export const KEY_BUDGET = 20;

/** The words before the first ` · `, to find a line whose counts change as it is read. */
export function lead(text: string): string {
  return text.split(DOT)[0] ?? text;
}

interface Fixtures {
  server: Server;
  /** A page signed in through the one-time link, on the first network's Overview. */
  app: Page;
}

export const test = base.extend<Fixtures>({
  // Playwright needs the destructuring even when a fixture uses no other fixture.
  // eslint-disable-next-line no-empty-pattern
  server: async ({}, use) => {
    const server = await startServer();
    await use(server);
    await server.stop();
  },
  baseURL: async ({ server }, use) => {
    await use(server.origin);
  },
  app: async ({ page, server }, use) => {
    const problems: string[] = [];
    page.on('pageerror', (error) => problems.push(`uncaught: ${error.message}`));
    page.on('console', (message) => {
      if (message.type() === 'error' && message.text().includes('Content Security Policy')) {
        problems.push(message.text());
      }
    });
    await page.goto(server.link);
    await page.waitForURL(`${server.origin}/n/${encodeURIComponent(network.name)}`);
    await settle(page);
    await use(page);
    expect(problems, 'no uncaught errors or CSP violations').toEqual([]);
  },
});

export { expect };

/** Waits until a screen has rendered and the embedded fonts are in. */
export async function settle(page: Page): Promise<void> {
  await expect(page.locator('#content > *').first()).toBeAttached();
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
}

/** Opens `path` in a fresh load, so the screen renders from the inline boot snapshot. */
export async function open(page: Page, path: string): Promise<void> {
  await page.goto(path);
  await settle(page);
}

/** Answers a change the fixture server refuses (it only replays) as the daemon would. */
export async function accept(route: Route): Promise<void> {
  const outcome: Outcome = { state: 'accepted', op: null };
  await route.fulfill({ json: outcome });
}

async function holdsFocus(target: Locator): Promise<boolean> {
  return target.evaluate(
    (element) => element === document.activeElement || element.contains(document.activeElement),
  );
}

/** Presses keys and counts every one, so a flow can prove it stays under the budget. */
export class Keys {
  count = 0;

  constructor(private readonly page: Page) {}

  async press(key: string): Promise<void> {
    this.count += 1;
    await this.page.keyboard.press(key);
  }

  /** Types `text` one key at a time; each character counts. */
  async type(text: string): Promise<void> {
    for (const character of text) await this.press(character);
  }

  /** Presses Tab until `target` holds focus, giving up at the budget. */
  async tabTo(target: Locator): Promise<void> {
    await target.waitFor();
    while (!(await holdsFocus(target))) {
      if (this.count >= KEY_BUDGET) {
        throw new Error(`no focus on ${target.toString()} within ${KEY_BUDGET} keys`);
      }
      await this.press('Tab');
    }
  }

  /** Uses the skip link, the first stop on every page, to jump past the frame. */
  async skipToContent(skipLabel: string): Promise<void> {
    await this.press('Tab');
    await expect(this.page.getByRole('link', { name: skipLabel })).toBeFocused();
    await this.press('Enter');
  }
}
