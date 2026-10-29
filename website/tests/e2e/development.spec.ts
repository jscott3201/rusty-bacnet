import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { release } from '../../src/lib/site.mjs';
import navigation from '../../src/data/navigation.json' with { type: 'json' };
type NavItem = { slug?: string; items?: NavItem[] };
/** A page's level-2 headings, read from its source. */
async function sectionHeadings(page: string) {
  const source = await readFile(new URL(`../../src/content/docs/${page}`, import.meta.url), 'utf8');
  return [...source.matchAll(/^## (.+)$/gm)].map(match => match[1].replace(/`/g, ''));
}
const base = '/rusty-bacnet/';

// Every path on the home page leads to a sidebar page that loads; the five
// readers the landing serves each have a card.
test('homepage paths reach a start page for each reader, and the contributor pages', async ({ page }) => {
  const navRoutes = new Set(navigation.flatMap(group => (group.items as NavItem[]).flatMap((item): NavItem[] => item.items ?? [item]).map(item => `${base}${item.slug}/`)));
  await page.goto(base);
  const paths = page.getByRole('region', { name: 'Start where you are.' });
  const cards = await paths.getByRole('link').evaluateAll(links => links.map(link => link.getAttribute('href') || ''));
  for (const reader of ['project/support/', 'start/python/', 'start/rust/', 'start/first-read/', 'tui/overview/', 'start/embedded/']) {
    expect(cards, reader).toContain(base + reader);
  }
  const more = await page.getByRole('region', { name: 'Then go further.' }).getByRole('link').evaluateAll(links => links.map(link => link.getAttribute('href') || ''));
  const contribute = await page.locator('.rb-home-contribute a').evaluateAll(links => links.map(link => link.getAttribute('href') || ''));
  expect(contribute).toEqual([base + 'development/build-from-source/', base + 'development/documentation/']);
  for (const href of [...cards, ...more, ...contribute]) {
    expect(navRoutes.has(href) || href === base + 'start/installation/', href).toBe(true);
  }
  await paths.getByRole('link', { name: /Install v/ }).click();
  await expect(page).toHaveURL(base + 'start/installation/');
  await expect(page.locator('main')).toContainText(`v${release}`);
  await page.goto(base);
  await paths.getByRole('link', { name: /Read with Python/ }).click();
  await expect(page).toHaveURL(base + 'start/python/');
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
});

test('search discovers canonical integration Network Port guidance and retains its scope', async ({ page, request }) => {
  await page.goto(base);
  await page.getByRole('button', { name: /Search/ }).first().click();
  await page.locator('.pagefind-ui__search-input').fill('Network Port');
  const result = page.locator('.pagefind-ui__result-link').filter({ hasText: 'Network Port and Number controls' });
  await expect(result.first()).toBeVisible();
  for (const href of await page.locator('.pagefind-ui__result-link').evaluateAll(links => links.map(link => link.getAttribute('href') || ''))) {
    expect(href).not.toContain('/development/network-number/');
  }
  await result.first().click();
  await expect(page).toHaveURL(/\/integrate\/network-ports\//);
  // The page and its export carry each section of the source.
  const headings = await sectionHeadings('integrate/network-ports.md');
  expect(headings.length).toBeGreaterThan(2);
  for (const heading of headings) {
    await expect(page.locator('main').getByRole('heading', { level: 2, name: heading, exact: true })).toBeAttached();
  }
  const text = await (await request.get(base + 'raw/integrate/network-ports.md')).text();
  for (const heading of headings) expect(text).toContain(`## ${heading}\n`);
  expect(text).toContain('https://jscott3201.github.io/rusty-bacnet/integrate/overview/');
});

test('choose-your-path routes firmware work to the embedded page', async ({ page, request }) => {
  await page.goto(base + 'start/choose-your-path/');
  await page.locator('.sl-markdown-content').getByRole('link', { name: 'Run on a microcontroller', exact: true }).click();
  await expect(page).toHaveURL(base + 'start/embedded/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Run on a microcontroller');
  const main = page.locator('main');
  for (const heading of await sectionHeadings('start/embedded.md')) {
    await expect(main.getByRole('heading', { level: 2, name: heading, exact: true })).toBeVisible();
  }
  const raw = await (await request.get(base + 'raw/start/embedded.md')).text();
  expect(raw).toContain('cargo run -p bacnet-router-core --features bip,mstp --example bip_mstp_router');
});
