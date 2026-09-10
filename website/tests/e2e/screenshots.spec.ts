import { test, expect } from '@playwright/test';
import { join } from 'node:path';
import { selectTheme, expectContainedPage } from './helpers';
const base = '/rusty-bacnet/';

test('capture actual production reading and installation states', async ({ page }, info) => {
  const directory = process.env.DOCS_SCREENSHOTS || info.outputPath('screenshots');
  for (const theme of ['light', 'dark'] as const) {
    await page.goto(base);
    await selectTheme(page, theme);
    await page.reload();
    const capture = async (label: string, fullPage = true) => {
      await expectContainedPage(page);
      await page.screenshot({ path: join(directory, `${info.project.name}-${theme}-${label}.png`), fullPage });
    };
    await expect(page.locator('.hero img')).toBeVisible();
    await capture('homepage');
    await capture('homepage-viewport', false);
    await page.goto(base + 'start/installation/');
    await capture('install-cli');
    await page.getByRole('tab', { name: 'Python', exact: true }).click();
    await capture('install-python-posix');
    await page.getByRole('tab', { name: 'Windows', exact: true }).click();
    await capture('install-python-windows');
    await page.goto(base + 'start/local-lab/');
    await capture('local-lab');
    await page.getByRole('button', { name: /Expand diagram/ }).click();
    await capture('diagram-fit', false);
    const slider = page.getByRole('dialog').getByRole('slider', { name: 'Zoom' });
    await slider.press('End');
    await expect(slider).toHaveValue('300');
    await capture('diagram-expanded', false);
    await page.keyboard.press('Escape');
    await page.goto(base + 'guides/configuration/');
    await capture('article');
  }
});
