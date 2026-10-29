import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import navigation from '../../src/data/navigation.json' with { type: 'json' };
const base = '/rusty-bacnet/';
const tui = navigation.find(group => group.label === 'Terminal UI')!.items;

test('the Terminal UI guide is one sidebar group, chained page to page', async ({ page }) => {
  expect(tui.map(item => item.slug)).toEqual(['tui/overview', 'tui/profiles', 'tui/discovery', 'tui/browser', 'tui/writes',
    'tui/watch', 'tui/network', 'tui/bacnet-sc', 'tui/mstp', 'tui/keys', 'tui/troubleshooting']);
  await page.goto(base + 'start/choose-your-path/');
  await page.locator('.sl-markdown-content').getByRole('link', { name: 'Use the terminal UI', exact: true }).click();
  await expect(page).toHaveURL(base + 'tui/overview/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Use the terminal UI');
  // Each page's NextStep leads to the next one in the sidebar.
  for (const [index, item] of tui.slice(0, -1).entries()) {
    await page.goto(base + item.slug + '/');
    await page.locator('a.rb-next-step').click();
    await expect(page, item.slug).toHaveURL(base + tui[index + 1].slug + '/');
  }
});

test('screenshots are optimized images with their manifest alt text', async ({ page, request }) => {
  const manifest = JSON.parse(await readFile(new URL('../../src/assets/tui/manifest.json', import.meta.url), 'utf8'));
  await page.goto(base + 'tui/writes/');
  const images = page.locator('main img');
  expect(await images.count()).toBeGreaterThan(8);
  for (const image of await images.all()) {
    const src = await image.getAttribute('src');
    expect(src, 'served from the build, not src/assets').toMatch(/^\/rusty-bacnet\/_astro\/[\w.-]+\.webp$/);
    const alt = await image.getAttribute('alt');
    expect(manifest.shots.map((shot: { alt: string }) => shot.alt)).toContain(alt);
    await expect(image).toHaveAttribute('width', /\d+/);
    await expect(image).toHaveAttribute('loading', 'lazy');
    expect((await request.get(src!)).status()).toBe(200);
  }
  const raw = await (await request.get(base + 'raw/tui/writes.md')).text();
  const shown = manifest.shots.filter((shot: { name: string }) => shot.name.startsWith('writes-'));
  expect(shown.length).toBeGreaterThan(8);
  for (const shot of shown) expect(raw).toContain(`*Screenshot: ${shot.alt}*`);
  expect(raw).not.toContain('assets/tui/');
});

test('the key reference is searchable and the MS/TP page keeps its safety order', async ({ page }) => {
  await page.goto(base);
  await page.getByRole('button', { name: /Search/ }).first().click();
  await page.locator('.pagefind-ui__search-input').fill('safe join');
  const result = page.locator('.pagefind-ui__result-link').filter({ hasText: 'Monitor and join MS/TP' }).first();
  await expect(result).toBeVisible();
  await result.click();
  await expect(page).toHaveURL(/\/rusty-bacnet\/tui\/mstp\//);
  const headings = await page.locator('main h2').allTextContents();
  expect(headings.indexOf('Listen first: the monitor')).toBeLessThan(headings.indexOf('Join directly: the participant'));
  await page.goto(base + 'tui/keys/');
  await expect(page.getByRole('cell', { name: 'Join the trunk', exact: true })).toBeVisible();
  await expect(page.getByRole('cell', { name: 'mstp listen SECS' })).toBeVisible();
});
