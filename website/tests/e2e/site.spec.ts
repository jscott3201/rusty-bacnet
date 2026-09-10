import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import navigation from '../../src/data/navigation.json' with { type: 'json' };
import { selectTheme, expectContainedPage } from './helpers';
const base = '/rusty-bacnet/';
const routes = ['', ...navigation.flatMap(group => group.items.map(item => `${item.slug}/`))];

test('all pages load directly under the project prefix without page overflow', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  for (const theme of ['light', 'dark'] as const) {
    await page.goto(base);
    await selectTheme(page, theme);
    for (const route of routes) {
    const response = await page.goto(base + route);
    expect(response?.status(), route).toBe(200);
    await expect(page.locator('h1').first()).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await expectContainedPage(page);
    const duplicateIds = await page.locator('[id]').evaluateAll(nodes => {
      const ids = nodes.map(node => node.id);
      return ids.filter((id, index) => ids.indexOf(id) !== index);
    });
    expect(duplicateIds, route).toEqual([]);
    for (const image of await page.locator('img:visible').all()) {
      await image.scrollIntoViewIfNeeded();
      await expect.poll(() => image.evaluate(node => node instanceof HTMLImageElement && node.complete && node.naturalWidth > 0), { message: route }).toBe(true);
    }
    expect((await page.reload())?.status(), route).toBe(200);
    }
  }
  expect(errors).toEqual([]);
});

test('installation uses native tabs and can reveal a search deep link', async ({ page }) => {
  await page.goto(base + 'start/installation/');
  await page.getByRole('tab', { name: 'Python', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Install the Python package' })).toBeVisible();
  await page.getByRole('tab', { name: 'Windows', exact: true }).click();
  await expect(page.getByText('py -3.13 -m venv .venv', { exact: false })).toBeVisible();
  await page.goto(base + 'start/installation/#install-the-python-package');
  await expect(page.getByRole('heading', { name: 'Install the Python package' })).toBeVisible();
});

test('diagrams expand, zoom, close on Escape, and restore focus', async ({ page }) => {
  await page.goto(base + 'start/local-lab/');
  const opener = page.getByRole('button', { name: /Expand diagram/ });
  await opener.click();
  const dialog = page.getByRole('dialog', { name: 'A complete read, on one machine' });
  await expect(dialog).toBeVisible();
  const slider = dialog.getByRole('slider', { name: 'Zoom' });
  await slider.fill('200');
  await expect(dialog.locator('[data-diagram-percent]')).toHaveText('200%');
  await dialog.getByRole('button', { name: 'Fit', exact: true }).click();
  await expect(slider).toHaveValue('100');
  await page.keyboard.press('Escape');
  await expect(dialog).not.toBeVisible();
  await expect(opener).toBeFocused();
});

test('native search returns a local-lab route under the project prefix', async ({ page }) => {
  await page.goto(base);
  await page.getByRole('button', { name: /Search/ }).first().click();
  const search = page.locator('.pagefind-ui__search-input');
  await search.fill('loopback');
  const result = page.locator('.pagefind-ui__result-link').filter({ hasText: /local.*lab/i }).first();
  await expect(result).toBeVisible();
  await result.click();
  await expect(page).toHaveURL(/\/rusty-bacnet\/start\/local-lab\//);
});

test('light and dark themes have no automated WCAG A/AA violations on every route', async ({ page }) => {
  for (const route of routes) {
    await page.goto(base + route);
    for (const theme of ['light', 'dark'] as const) {
      await selectTheme(page, theme);
      const scan = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
      expect(scan.violations, `${route} / ${theme}`).toEqual([]);
    }
  }
});

test('public SVG, raw guide, lab script, and 404 output exist', async ({ request }) => {
  for (const asset of ['diagrams/local-lab.svg', 'examples/loopback_lab.py', 'raw/start/local-lab.md', 'llms.txt', '404.html']) {
    const response = await request.get(base + asset);
    expect(response.ok(), asset).toBe(true);
  }
});

test('local navigation and source links are not accidentally rooted outside the project', async ({ page }) => {
  for (const route of routes) {
    await page.goto(base + route);
    const bad = await page.locator('a[href^="/"]').evaluateAll(links => links.map(a => a.getAttribute('href') || '').filter(href => !href.startsWith('/rusty-bacnet/')));
    expect(bad, route).toEqual([]);
  }
});
