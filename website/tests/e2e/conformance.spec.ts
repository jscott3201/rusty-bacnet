import { test, expect } from '@playwright/test';
import support from '../../src/data/support.json' with { type: 'json' };
const base = '/rusty-bacnet/';

test('the capability tables render a row per generated entry and export as Markdown', async ({ page, request }) => {
  await page.goto(base + 'project/support/');
  await page.locator('main').getByRole('link', { name: 'capability tables', exact: true }).first().click();
  await expect(page).toHaveURL(base + 'project/conformance/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Capability tables');
  const main = page.locator('.sl-markdown-content');
  const rows = (id: string) => main.locator(`#table-${id} tbody tr`);
  await expect(rows('objects')).toHaveCount(support.objectTypes.length);
  const kinds = ['confirmed', 'unconfirmed'];
  for (const kind of kinds) {
    await expect(rows(`services-${kind}`)).toHaveCount(support.services.filter(service => service.kind === kind).length);
  }
  await expect(rows('links')).toHaveCount(support.dataLinks.length);
  // Each table is a keyboard-focusable scroll region named by its caption.
  for (const id of ['objects', ...kinds.map(kind => `services-${kind}`), 'links']) {
    const region = main.locator(`.rb-table-scroll:has(#table-${id})`);
    await expect(region).toHaveAttribute('tabindex', '0');
    await expect(region).toHaveAttribute('aria-label', (await main.locator(`#table-${id} caption`).textContent()) ?? '');
  }
  await expect(rows('services-confirmed').filter({ hasText: 'ReadProperty' }).first().locator('td').nth(3)).toHaveText('Always');
  await main.getByRole('link', { name: 'what a result proves', exact: true }).click();
  await expect(page).toHaveURL(base + 'project/support/#understand-what-a-result-proves');

  const raw = await request.get(base + 'raw/project/conformance.md');
  expect(raw.status()).toBe(200);
  const body = await raw.text();
  expect(body).not.toContain('<SupportTable');
  expect(body).toContain('| Object type | Type number | CreateObject | DeleteObject |');
});
