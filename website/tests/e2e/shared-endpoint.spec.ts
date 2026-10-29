import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
const base = '/rusty-bacnet/';

test('integration journey exposes the exact runnable shared-endpoint source and downloads', async ({ page, request }) => {
  const source = await readFile(new URL('../../examples/rust/shared-endpoint/src/main.rs', import.meta.url), 'utf8');
  const manifest = await readFile(new URL('../../examples/rust/shared-endpoint/Cargo.toml', import.meta.url), 'utf8');
  await page.goto(base + 'integrate/overview/');
  await page.locator('main').getByRole('link', { name: 'Shared endpoints', exact: true }).click();
  await expect(page).toHaveURL(base + 'integrate/shared-endpoints/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Compose a shared endpoint');
  const main = page.locator('.sl-markdown-content');
  await expect(main.locator('pre code').filter({ hasText: 'async fn main()' })).toHaveText(source.trimEnd(), { useInnerText: true });
  await expect(main.locator('pre code').filter({ hasText: '[package]' })).toHaveText(manifest.trimEnd(), { useInnerText: true });
  for (const [label, path, expected] of [
    ['Cargo.toml', 'Cargo.toml', manifest],
    ['main.rs', 'src/main.rs', source],
  ]) {
    const href = base + 'examples/rust/shared-endpoint/' + path;
    await expect(main.getByRole('link', { name: label, exact: true })).toHaveAttribute('href', href);
    const download = await request.get(href);
    expect(download.status()).toBe(200);
    expect(await download.text()).toBe(expected);
  }
  const raw = await request.get(base + 'raw/integrate/shared-endpoints.md');
  expect(raw.status()).toBe(200);
  const body = await raw.text();
  expect(body.match(/^```rust\n([\s\S]*?)^```/m)?.[1]).toBe(source);
  expect(body.match(/^```toml\n([\s\S]*?)^```/m)?.[1]).toBe(manifest);
  // The expected output the page shows is the program's own last line.
  const format = source.match(/"(Rebound endpoint: \{\}; owner running=)\{\}"/)?.[1];
  expect(format).toBeTruthy();
  expect(body).toContain(format!.replace('{}', '127.0.0.1:<assigned-port>'));
});
