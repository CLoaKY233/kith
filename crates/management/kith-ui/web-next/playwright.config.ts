import { existsSync } from 'node:fs';
import { defineConfig, devices } from '@playwright/test';

const shell = '/Users/cloaky/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';

export default defineConfig({
  testDir: 'e2e',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  reporter: 'list',
  use: {
    baseURL: 'http://localhost:41695',
    trace: 'retain-on-failure',
    launchOptions: existsSync(shell) ? { executablePath: shell } : {},
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'bash e2e/serve.sh',
    url: 'http://localhost:41695/login',
    timeout: 600_000,
    reuseExistingServer: false,
    stdout: 'pipe',
  },
});
