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

test('choose-your-path routes firmware work to the embedded page', async ({ page, request }) => {
  await page.goto(base + 'start/choose-your-path/');
  await page.locator('.sl-markdown-content').getByRole('link', { name: 'Run on a microcontroller', exact: true }).click();
  await expect(page).toHaveURL(base + 'start/embedded/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Run on a microcontroller');
  const main = page.locator('main');
  await expect(main.getByRole('heading', { name: 'Adapter rules', exact: true })).toBeVisible();
  await expect(main).toContainText('Merge deadlines with bacnet_time::earliest.');
  await expect(main.getByRole('cell', { name: 'a whole MS/TP device with Who-Is and ReadProperty' })).toBeVisible();
  const raw = await (await request.get(base + 'raw/start/embedded.md')).text();
  expect(raw).toContain('cargo run -p bacnet-router-core --features bip,mstp --example bip_mstp_router');
});
