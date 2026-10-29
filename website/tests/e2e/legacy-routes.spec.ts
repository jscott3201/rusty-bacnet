import { test, expect } from '@playwright/test';
import legacyRoutes from '../../src/data/legacy-routes.json' with { type: 'json' };
const base = '/rusty-bacnet/';

test('legacy guides retain every section and raw link without duplicating the guide index', async ({ page, request }) => {
  const index = await (await request.get(base + 'llms.txt')).text();
  for (const route of legacyRoutes) {
    expect((await page.goto(base + route.from + '/'))?.status()).toBe(200);
    const main = page.locator('.sl-markdown-content');
    await expect(main).toContainText('This guide has moved.');
    await expect(main.locator(`a[href="${base}${route.to}/"]`)).toHaveCount(1);
    // Starlight excludes these compatibility pages from Pagefind's body index.
    await expect(page.locator('[data-pagefind-body]')).toHaveCount(0);
    expect(index).not.toContain(base + 'raw/' + route.from + '.md');
    const raw = await request.get(base + 'raw/' + route.from + '.md');
    expect(raw.status()).toBe(200);
    const text = await raw.text();
    expect(text).toContain('https://jscott3201.github.io' + base + route.to + '/');
    for (const section of route.sections) {
      await expect(main.locator('#' + section.anchor)).toHaveCount(1);
      expect(text).toContain('https://jscott3201.github.io' + base + section.target);
      const target = await request.get(base + section.target);
      expect(target.status()).toBe(200);
      const fragment = section.target.split('#')[1];
      expect(await page.evaluate(({ html, fragment }) => !!new DOMParser().parseFromString(html, 'text/html').getElementById(fragment),
        { html: await target.text(), fragment })).toBe(true);
    }
  }
});

test('saved source-build fragment reaches the contributor guide without JavaScript', async ({ browser, baseURL }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, baseURL });
  try {
    const page = await context.newPage();
    await page.goto(base + 'development/overview/#build-a-source-checkout');
    await expect(page.locator('#build-a-source-checkout')).toBeInViewport();
    await page.getByRole('link', { name: 'Continue: Build a source checkout', exact: true }).click();
    await expect(page).toHaveURL(base + 'development/build-from-source/#build-a-source-checkout');
    await expect(page.locator('#build-a-source-checkout')).toBeInViewport();
    await expect(page.locator('main')).toContainText('git rev-parse HEAD');
    expect((await page.reload())?.status()).toBe(200);
  } finally {
    await context.close();
  }
});

test('native sidebar separates integration and contributor destinations', async ({ page }) => {
  await page.goto(base + 'integrate/overview/');
  const menu = page.getByRole('button', { name: 'Menu', exact: true });
  if (await menu.isVisible()) await menu.click();
  const sidebar = page.locator('#starlight__sidebar');
  await expect(sidebar.getByRole('link', { name: 'Integration overview', exact: true })).toHaveAttribute('href', base + 'integrate/overview/');
  await sidebar.getByRole('link', { name: 'Build from source', exact: true }).click();
  await expect(page).toHaveURL(base + 'development/build-from-source/');
  if (await menu.isVisible()) await expect(sidebar).not.toBeVisible();
});
