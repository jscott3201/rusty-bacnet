import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
const base = '/rusty-bacnet/';

test('security page shows the exact policy program and serves its downloads', async ({ page, request }) => {
  const source = await readFile(new URL('../../examples/rust/sc-operator-policy/src/main.rs', import.meta.url), 'utf8');
  const manifest = await readFile(new URL('../../examples/rust/sc-operator-policy/Cargo.toml', import.meta.url), 'utf8');
  await page.goto(base + 'integrate/overview/');
  await page.locator('main').getByRole('link', { name: 'Restrict who can change a device', exact: true }).first().click();
  await expect(page).toHaveURL(base + 'integrate/security/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Restrict who can change a device');
  const main = page.locator('.sl-markdown-content');
  await expect(main.locator('pre code').filter({ hasText: 'impl ServerPolicy for OperatorOnly' })).toHaveText(source.trimEnd(), { useInnerText: true });
  await expect(main.locator('pre code').filter({ hasText: '[package]' })).toHaveText(manifest.trimEnd(), { useInnerText: true });
  for (const [label, path, expected] of [
    ['Cargo.toml', 'Cargo.toml', manifest],
    ['main.rs', 'src/main.rs', source],
  ]) {
    const href = base + 'examples/rust/sc-operator-policy/' + path;
    await expect(main.getByRole('link', { name: label, exact: true })).toHaveAttribute('href', href);
    const download = await request.get(href);
    expect(download.status()).toBe(200);
    expect(await download.text()).toBe(expected);
  }
  await main.getByRole('link', { name: 'example program', exact: true }).click();
  await expect(page).toHaveURL(base + 'integrate/security/#example-writes-and-dcc-from-one-certificate-name');
  const raw = await request.get(base + 'raw/integrate/security.md');
  expect(raw.status()).toBe(200);
  const body = await raw.text();
  expect(body.match(/^```rust\n([\s\S]*?)^```/m)?.[1]).toBe(source);
  expect(body).toContain('DCC without a certificate: denied');
});
