import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import navigation from '../../src/data/navigation.json' with { type: 'json' };
const base = '/rusty-bacnet/';
const origin = 'https://jscott3201.github.io';
const routes = ['', ...navigation.flatMap(group => group.items.map(item => `${item.slug}/`))];

test('canonical, sitemap, all local links and assets resolve under the project prefix', async ({ page, request }) => {
  const targets = new Map<string, Set<string>>();
  for (const route of routes) {
    await page.goto(base + route);
    await expect(page.locator('link[rel="canonical"]')).toHaveAttribute('href', origin + base + route);
    const urls = await page.locator('a[href], img[src], script[src], link[href]').evaluateAll(nodes => nodes.map(node => {
      return node.getAttribute('href') || node.getAttribute('src') || '';
    }));
    for (const value of urls) {
      const url = new URL(value, page.url());
      if (url.origin !== new URL(page.url()).origin) continue;
      expect(url.pathname, `${route}: ${value}`).toMatch(/^\/rusty-bacnet\//);
      const hashes = targets.get(url.pathname) || new Set<string>();
      if (url.hash) hashes.add(decodeURIComponent(url.hash.slice(1)));
      targets.set(url.pathname, hashes);
    }
  }
  for (const [path, hashes] of targets) {
    const response = await request.get(path);
    expect(response.status(), path).toBe(200);
    if (hashes.size) {
      const missing = await page.evaluate(({ html, ids }) => {
        const doc = new DOMParser().parseFromString(html, 'text/html');
        return ids.filter(id => !doc.getElementById(id));
      }, { html: await response.text(), ids: [...hashes] });
      expect(missing, path).toEqual([]);
    }
  }
  const index = await request.get(base + 'sitemap-index.xml');
  expect(index.status()).toBe(200);
  const sitemap = await request.get(base + 'sitemap-0.xml');
  expect(sitemap.status()).toBe(200);
  const urls = await page.evaluate(xml => Array.from(new DOMParser().parseFromString(xml, 'application/xml').querySelectorAll('loc'), node => node.textContent), await sitemap.text());
  for (const route of routes) expect(urls).toContain(origin + base + route);
  for (const url of urls) expect(url).toMatch(/^https:\/\/jscott3201.github.io\/rusty-bacnet\//);
  expect(await index.text()).toContain(origin + base + 'sitemap-0.xml');
});

test('download, MDX source view, Markdown exports and llms are source-derived', async ({ request, page }) => {
  const source = await readFile(new URL('../../examples/python/loopback_lab.py', import.meta.url), 'utf8');
  const download = await request.get(base + 'examples/loopback_lab.py');
  expect(await download.text()).toBe(source);
  const llms = await request.get(base + 'llms.txt');
  const index = await llms.text();
  expect(index.match(/^- \[/gm)).toHaveLength(19);
  for (const route of routes.filter(Boolean)) {
    const rawPath = `raw/${route.slice(0, -1)}.md`;
    expect(index).toContain(origin + base + rawPath);
    const raw = await request.get(base + rawPath);
    expect(raw.status()).toBe(200);
    const body = await raw.text();
    expect(body).toMatch(/^# /);
    expect(body).not.toMatch(/\]\(\/(?!rusty-bacnet\/)/);
    expect(body).not.toMatch(/<(?:Tabs|TabItem|Code|Diagram|NextStep|DownloadList)\b/);
    if (route === 'start/local-lab/') expect(body).toContain(source.trim());
  }
  await page.goto(base + 'start/local-lab/');
  await page.getByText('Show the complete loopback_lab.py script', { exact: true }).click();
  await expect(page.locator('details code')).toHaveText(source.trim(), { useInnerText: true });
});

test('unknown route returns a useful custom 404 with working local recovery', async ({ page }) => {
  const response = await page.goto(base + 'missing-guide-for-404-test/');
  expect(response?.status()).toBe(404);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('That route is not in these guides.');
  await page.getByRole('link', { name: 'Choose your path', exact: true }).click();
  await expect(page).toHaveURL(/\/rusty-bacnet\/start\/choose-your-path\/$/);
});
