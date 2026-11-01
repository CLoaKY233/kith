// End-to-end tests against `kith-ui --fixture` (spec §1.4, §11). Each test gets its own
// server, so changes and the one-time link never leak between tests. Build the page and
// then the binary first: `npm run build && cargo build -p kith-ui`.

import { defineConfig, devices } from '@playwright/test';

const ci = Boolean(process.env['CI']);

export default defineConfig({
  testDir: 'tests/playwright',
  globalSetup: './tests/playwright/global-setup.ts',
  fullyParallel: true,
  forbidOnly: ci,
  // A flaky test is a bug to find, not something to retry.
  retries: 0,
  workers: ci ? 2 : undefined,
  reporter: ci ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    locale: 'en-GB',
    timezoneId: 'UTC',
    // A missing control fails fast with its name instead of at the test timeout.
    actionTimeout: 10_000,
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
