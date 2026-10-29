import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import navigation from '../../src/data/navigation.json' with { type: 'json' };
import { selectTheme, expectContainedPage } from './helpers';
const base = '/rusty-bacnet/';
// A sidebar entry: a page, or a nested group of pages.
type NavItem = { slug?: string; items?: NavItem[] };
const routes = ['', ...navigation.flatMap(group => (group.items as NavItem[]).flatMap((item): NavItem[] => item.items ?? [item]).map(item => `${item.slug}/`))];

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

// One test per route, so each scan pair stays well inside the test timeout as
// the site grows, and the two workers can share the routes.
test.describe('light and dark themes have no automated WCAG A/AA violations', () => {
  test.describe.configure({ mode: 'parallel' });
  for (const route of routes) {
    test(`on ${base}${route}`, async ({ page }) => {
      await page.goto(base + route);
      // Every tab, nested ones too, so each panel is scanned while shown.
      const tabs = await page.locator('[role="tab"]').count();
      for (const theme of ['light', 'dark'] as const) {
        await selectTheme(page, theme);
        const scan = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
        expect(scan.violations, `${route} / ${theme}`).toEqual([]);
        for (let index = 0; index < tabs; index++) {
          const tab = page.locator('[role="tab"]').nth(index);
          // Show the panels that hold a nested tab first, through their own tabs.
          for (const control of await tab.evaluate(node => {
            const ids: string[] = [];
            for (let at = node.parentElement; at; at = at.parentElement) {
              if (at.getAttribute('role') === 'tabpanel') ids.unshift(at.getAttribute('aria-labelledby') || '');
            }
            return ids;
          })) await page.locator(`[id="${control}"]`).click();
          await tab.click();
          await expect(tab).toHaveAttribute('aria-selected', 'true');
          await expectContainedPage(page);
          // A block in a newly shown panel gets its tabindex back
          // (src/scripts/tab-code-focus.js) on the next frame.
          await expect.poll(() => page.locator('pre:visible').evaluateAll(blocks =>
            blocks.filter(block => block.scrollWidth > block.clientWidth && !block.hasAttribute('tabindex')).length)).toBe(0);
          const panel = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
          expect(panel.violations, `${route} / ${theme} / tab ${await tab.textContent()}`).toEqual([]);
        }
      }
    });
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

// Generated reference pages (DocsMarkdownContent.astro) weigh less in search:
// a task phrase ranks the guides first, and an API name still finds its page.
test('search ranks guides above generated reference pages but still finds API names', async ({ page }) => {
  await page.goto(base);
  const ranked = await page.evaluate(async ({ bundle, queries }) => {
    const pagefind = await import(bundle);
    await pagefind.init();
    const out: Record<string, string[]> = {};
    for (const query of queries) {
      const { results } = await pagefind.search(query);
      out[query] = await Promise.all(results.slice(0, 8).map(async (result: { data: () => Promise<{ url: string }> }) => (await result.data()).url));
    }
    return out;
  }, { bundle: base + 'pagefind/pagefind.js', queries: ['read property', 'subscribe', 'write priority', 'cov_counters'] });
  const generated = /\/reference\/(cli|rust|python)\b/;
  for (const query of ['read property', 'subscribe', 'write priority']) {
    expect(ranked[query].slice(0, 3).filter(url => generated.test(url)), query).toEqual([]);
  }
  expect(ranked.cov_counters).toContain(base + 'reference/python/server/');
});
