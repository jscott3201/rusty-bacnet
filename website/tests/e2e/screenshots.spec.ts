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
    await page.goto(base + 'connect/bacnet-ip/');
    await capture('article');
    for (const route of ['overview', 'shared-endpoints', 'network-ports']) {
      await page.goto(base + 'integrate/' + route + '/');
      await capture('integrate-' + route);
    }
    await page.goto(base + 'development/overview/#build-a-source-checkout');
    await capture('legacy-source-bookmark', false);
    await page.getByRole('link', { name: 'Continue: Build a source checkout', exact: true }).click();
    await capture('contributor-source-build', false);
    const menu = page.getByRole('button', { name: 'Menu', exact: true });
    if (await menu.isVisible()) await menu.click();
    const sidebar = page.locator('#starlight__sidebar');
    for (const label of ['Start with v0.15', 'Connect to a network', 'Discover, read and write', 'Terminal UI']) {
      const group = sidebar.locator('details').filter({ has: page.locator('summary').filter({ hasText: new RegExp('^' + label + '$') }) }).first();
      if (await group.getAttribute('open') !== null) await group.locator('summary').first().click();
    }
    await sidebar.getByRole('link', { name: 'Build from source', exact: true }).scrollIntoViewIfNeeded();
    await capture('integration-contributor-navigation', false);
    if (await menu.isVisible()) await page.keyboard.press('Escape');
    for (const route of ['overview', 'writes']) {
      await page.goto(base + 'tui/' + route + '/');
      await capture('tui-' + route);
    }
  }
});
