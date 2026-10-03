import { test, expect } from '@playwright/test';
import { release } from '../../src/lib/site.mjs';
const base = '/rusty-bacnet/';

test('homepage routes release readers and current-source builders separately', async ({ page }) => {
  await page.goto(base);
  const versions = page.getByRole('region', { name: 'Choose your documentation version' });
  await versions.getByRole('link', { name: /Build with the current APIs/ }).click();
  await expect(page).toHaveURL(base + 'development/overview/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Build with current development');
  await page.locator('main').getByRole('link', { name: 'Shared endpoints', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Check requester and responder scope separately', exact: true })).toBeVisible();
  await expect(page.locator('main')).toContainText('no source Reporter configuration');
  await page.locator('.sl-markdown-content').getByRole('link', { name: 'Current development', exact: true }).click();
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
  await expect(page.locator('main')).toContainText('Unreleased source behavior');
  await expect(page.locator('main')).toContainText('no proactive startup announcement');
  const raw = await request.get(base + 'raw/development/network-number.md');
  const text = await raw.text();
  expect(text).toContain('Unreleased source behavior');
  expect(text).toContain('Register a NORMAL B/IP receiving port');
  expect(text).toContain('https://jscott3201.github.io/rusty-bacnet/development/overview/');
});
