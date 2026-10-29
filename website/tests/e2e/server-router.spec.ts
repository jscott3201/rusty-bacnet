import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
const base = '/rusty-bacnet/';
const example = (path: string) => readFile(new URL('../../examples/' + path, import.meta.url), 'utf8');

// Each page shows the exact example files, serves them as downloads, and
// keeps them byte for byte in its raw export.
const pages = [
  {
    title: 'Serve objects from your application',
    path: 'integrate/server/',
    hasDev: false,
    files: [
      ['Cargo.toml', 'rust/server-device/Cargo.toml', 'examples/rust/server-device/Cargo.toml', '[package]', 'toml'],
      ['main.rs', 'rust/server-device/src/main.rs', 'examples/rust/server-device/src/main.rs', 'async fn main()', 'rust'],
      ['server_device.py', 'python/server_device.py', 'examples/server_device.py', 'async def main()', 'python'],
    ],
  },
  {
    title: 'Route between networks',
    path: 'integrate/router/',
    hasDev: false,
    files: [
      ['Cargo.toml', 'rust/bip-router/Cargo.toml', 'examples/rust/bip-router/Cargo.toml', '[package]', 'toml'],
      ['main.rs', 'rust/bip-router/src/main.rs', 'examples/rust/bip-router/src/main.rs', 'async fn main()', 'rust'],
      ['bip_router.py', 'python/bip_router.py', 'examples/bip_router.py', 'async def main()', 'python'],
    ],
  },
] as const;

test('the integration overview links the server and router pages, which show their exact programs', async ({ page, request }) => {
  for (const { title, path, hasDev, files } of pages) {
    await page.goto(base + 'integrate/overview/');
    await page.locator('main').getByRole('link', { name: title, exact: true }).first().click();
    await expect(page).toHaveURL(base + path);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(title);
    const main = page.locator('.sl-markdown-content');
    await expect(main.getByRole('heading', { level: 2, name: 'Next steps' })).toBeVisible();
    const raw = await request.get(base + 'raw/' + path.replace(/\/$/, '.md'));
    expect(raw.status()).toBe(200);
    const body = await raw.text();
    for (const [label, source, href, marker, lang] of files) {
      const text = await example(source);
      // Each program sits in its language's tab; the manifest beside the Rust one.
      await main.getByRole('tab', { name: lang === 'python' ? 'Python' : 'Rust', exact: true }).click();
      await expect(main.locator('pre code').filter({ hasText: marker }).filter({ hasText: text.split('\n')[0] }))
        .toHaveText(text.trimEnd(), { useInnerText: true });
      await expect(main.getByRole('link', { name: label, exact: true })).toHaveAttribute('href', base + href);
      const download = await request.get(base + href);
      expect(download.status()).toBe(200);
      expect(await download.text()).toBe(text);
      expect(body.match(new RegExp('^```' + lang + '\\n([\\s\\S]*?)^```', 'm'))?.[1]).toBe(text);
    }
    if (hasDev) {
      await main.getByRole('link', { name: 'their own section', exact: true }).click();
      await expect(page).toHaveURL(base + path + '#on-dev-after-the-release');
    } else {
      await expect(main.locator('#on-dev-after-the-release')).toHaveCount(0);
    }
  }
});
