// The Console's flows (setup, sign-in, people, device codes) against mocked endpoints: the
// built page under `vite preview`, every /api call answered by the test, and Chrome's
// virtual authenticator standing in for Touch ID. No kith-ui binary is needed; run
// `npm run build` first.

import { defineConfig, devices } from '@playwright/test';

const ci = Boolean(process.env['CI']);
const port = 4179;

export default defineConfig({
  testDir: 'tests/mocked',
  fullyParallel: true,
  forbidOnly: ci,
  retries: 0,
  reporter: ci ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    // Passkeys need a secure context and a domain for the rp id; localhost is both.
    baseURL: `http://localhost:${port}`,
    locale: 'en-GB',
    timezoneId: 'UTC',
    actionTimeout: 10_000,
    trace: 'retain-on-failure',
  },
  webServer: {
    command: `npx vite preview --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: !ci,
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
