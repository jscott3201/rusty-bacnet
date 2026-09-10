import { defineConfig } from '@playwright/test';
const port = Number(process.env.DOCS_TEST_PORT || 46327);
const origin = `http://127.0.0.1:${port}`;
export default defineConfig({
  testDir: './tests/e2e',
  timeout: 90_000,
  retries: 0,
  workers: 2,
  outputDir: process.env.DOCS_TEST_OUTPUT || 'test-results',
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: origin,
    actionTimeout: 10_000,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    { name: 'desktop', use: { browserName: 'chromium', viewport: { width: 1440, height: 1000 } } },
    { name: 'tablet', use: { browserName: 'chromium', viewport: { width: 768, height: 1024 } } },
    { name: 'phone', use: { browserName: 'chromium', viewport: { width: 390, height: 844 }, hasTouch: true } },
    { name: 'narrow', use: { browserName: 'chromium', viewport: { width: 320, height: 800 }, hasTouch: true } },
    ...(process.env.DOCS_WEBKIT ? [{ name: 'webkit', testMatch: '**/smoke.spec.ts', use: { browserName: 'webkit' as const, viewport: { width: 390, height: 844 } } }] : []),
  ],
  webServer: {
    command: 'node scripts/test-server.mjs',
    env: { DOCS_TEST_PORT: String(port) },
    url: `${origin}/rusty-bacnet/`,
    reuseExistingServer: false,
    gracefulShutdown: { signal: 'SIGTERM', timeout: 5_000 },
    timeout: 30_000,
  },
});
