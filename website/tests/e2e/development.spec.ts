import { test, expect } from '@playwright/test';
import { release } from '../../src/lib/site.mjs';
const base = '/rusty-bacnet/';

test('homepage routes installers and integrators separately', async ({ page }) => {
  await page.goto(base);
  const versions = page.getByRole('region', { name: 'Choose where to start' });
  await versions.getByRole('link', { name: /Build with the APIs/ }).click();
  await expect(page).toHaveURL(base + 'development/overview/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Build and integrate');
  await page.locator('main').getByRole('link', { name: 'Shared endpoints', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Check requester and responder scope separately', exact: true })).toBeVisible();
  await expect(page.locator('main')).toContainText('no source Reporter configuration');
  await page.locator('.sl-markdown-content').getByRole('link', { name: 'Build and integrate', exact: true }).click();
  await expect(page).toHaveURL(base + 'development/overview/');
  await page.goto(base);
  await versions.getByRole('link', { name: /Start with a released tool/ }).click();
  await expect(page).toHaveURL(base + 'start/installation/');
  await expect(page.locator('main')).toContainText(`v${release} release assets`);
});

test('search discovers development Network Port guidance and retains its scope', async ({ page, request }) => {
  await page.goto(base);
  await page.getByRole('button', { name: /Search/ }).first().click();
  await page.locator('.pagefind-ui__search-input').fill('Network Port');
  const result = page.locator('.pagefind-ui__result-link').filter({ hasText: 'Network Port and Number controls' });
  await expect(result.first()).toBeVisible();
  await result.first().click();
  await expect(page).toHaveURL(/\/development\/network-number\//);
  await expect(page.locator('main')).toContainText('They make no announcement at startup');
  const raw = await request.get(base + 'raw/development/network-number.md');
  const text = await raw.text();
  expect(text).toContain('A Network Port object, a transport');
  expect(text).toContain('Register a B/IP receiving port');
  expect(text).toContain('https://jscott3201.github.io/rusty-bacnet/development/overview/');
});
