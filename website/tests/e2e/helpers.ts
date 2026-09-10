import { expect, type Page } from '@playwright/test';

export async function selectTheme(page: Page, theme: 'light' | 'dark' | 'auto') {
  const picker = page.getByRole('combobox', { name: 'Select theme' });
  const menu = page.getByRole('button', { name: 'Menu', exact: true });
  const needsMenu = await picker.count() === 0;
  if (needsMenu) await menu.click();
  await picker.selectOption(theme);
  await expect(picker).toHaveValue(theme);
  if (needsMenu) await menu.click();
  if (theme !== 'auto') await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
}

export async function expectContainedPage(page: Page) {
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
}
