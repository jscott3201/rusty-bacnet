import { test, expect } from '@playwright/test';
const base = '/rusty-bacnet/';
const reference = 'catalogue reference configuration';

test('the PICS task page leads to the sample, which opens with its banner on the page and in the export', async ({ page, request }) => {
  await page.goto(base + 'project/conformance/');
  await page.locator('main').getByRole('link', { name: 'generate a PICS', exact: true }).first().click();
  await expect(page).toHaveURL(base + 'integrate/pics/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Generate a PICS for your device');
  const main = page.locator('.sl-markdown-content');
  // One program, examples/rust/generate_pics.rs: the PICS takes the product
  // and protocol details from the Device (#2125, #2137), so the program sets
  // no protocol property itself.
  const programs = main.locator('pre').filter({ hasText: 'server.generate_pics(&pics_config).await' });
  await expect(programs).toHaveCount(1);
  await expect(programs.filter({ hasText: 'use bacnet_objects::DeviceIdentity;' })).toHaveCount(1);
  await expect(programs.filter({ hasText: 'PropertyIdentifier::PROTOCOL_REVISION' })).toHaveCount(0);
  await expect(main.getByRole('table').filter({ hasText: 'PicsConfig' })).toHaveCount(1);

  await main.getByRole('link', { name: 'sample PICS', exact: true }).first().click();
  await expect(page).toHaveURL(base + 'project/sample-pics/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(`Sample PICS: the ${reference}`);
  // The banner is the first thing in the content, before the generator's output.
  const banner = main.locator('aside').first();
  await expect(banner).toContainText(`Generated sample for the ${reference} only`);
  await expect(banner).toContainText('not a claim for any product or device');
  const generated = main.getByRole('heading', { level: 2, name: 'BACnet Protocol Implementation Conformance Statement (PICS)' });
  await expect(generated).toBeVisible();
  expect(await banner.evaluate((aside, heading) => Boolean(aside.compareDocumentPosition(heading!) & Node.DOCUMENT_POSITION_FOLLOWING),
    await generated.elementHandle())).toBe(true);
  await expect(main.getByRole('heading', { level: 3, name: 'Supported Services' })).toBeVisible();
  await banner.getByRole('link', { name: 'generate one from your own configuration' }).click();
  await expect(page).toHaveURL(base + 'integrate/pics/');

  // The raw Markdown carries the title and the banner ahead of the PICS.
  const raw = await request.get(base + 'raw/project/sample-pics.md');
  expect(raw.status()).toBe(200);
  const body = await raw.text();
  expect(body.startsWith(`# Sample PICS: the ${reference}\n\n:::caution[Generated sample for the ${reference} only]`)).toBe(true);
  expect(body.indexOf(':::caution')).toBeLessThan(body.indexOf('## BACnet Protocol Implementation Conformance Statement (PICS)'));
});
