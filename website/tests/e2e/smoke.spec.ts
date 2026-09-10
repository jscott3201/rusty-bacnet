import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { selectTheme, expectContainedPage } from './helpers';
const base = '/rusty-bacnet/';

test('engine smoke: deep tabs, keyboard-scrollable code, Pagefind and modal', async ({ page }) => {
  await page.goto(base + 'start/installation/#python-on-windows');
  await expect(page.locator('#python-on-windows')).toBeFocused();
  const code = page.getByRole('region', { name: 'powershell code example' }).last();
  await expect(code).toHaveAttribute('tabindex', '0');
  // Reach the code using Tab from the deep-linked heading, not DOM focus mutation.
  for (let step = 0; step < 12 && !await code.evaluate(node => node === document.activeElement); step++) {
    await page.keyboard.press('Tab');
  }
  await expect(code).toBeFocused();
  await code.press('ArrowRight', { delay: 100 });
  if (await code.evaluate(node => node.scrollWidth > node.clientWidth)) {
    await expect.poll(() => code.evaluate(node => node.scrollLeft)).toBeGreaterThan(0);
  }
  await expectContainedPage(page);
  await selectTheme(page, 'dark');
  expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
  await page.getByRole('button', { name: /Search/ }).first().click();
  const search = page.locator('.pagefind-ui__search-input');
  await search.fill('loopback');
  const result = page.locator('.pagefind-ui__result-link').filter({ hasText: /local.*lab/i }).first();
  await expect(result).toBeVisible();
  await result.click();
  await expect(page).toHaveURL(/\/rusty-bacnet\/start\/local-lab\//);
  const opener = page.getByRole('button', { name: /Expand diagram/ });
  await opener.click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await dialog.getByRole('slider', { name: 'Zoom' }).press('End');
  await expect(dialog.locator('output')).toHaveText('300%');
  await page.keyboard.press('Escape');
  await expect(opener).toBeFocused();
  await page.goto(base + 'guides/configuration/#bacnetip-settings');
  const table = page.getByRole('region', { name: 'Scrollable reference table' }).first();
  await expect(table.getByRole('table')).toBeVisible();
  await expect(table).toHaveAttribute('tabindex', '0');
  for (let step = 0; step < 12 && !await table.evaluate(node => node === document.activeElement); step++) {
    await page.keyboard.press('Tab');
  }
  await expect(table).toBeFocused();
  await table.press('ArrowRight', { delay: 100 });
  if (await table.evaluate(node => node.scrollWidth > node.clientWidth)) {
    await expect.poll(() => table.evaluate(node => node.scrollLeft)).toBeGreaterThan(0);
  }
  await expectContainedPage(page);
});
