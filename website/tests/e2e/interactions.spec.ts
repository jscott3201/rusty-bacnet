import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { readFile } from 'node:fs/promises';
import { selectTheme, expectContainedPage } from './helpers';
const base = '/rusty-bacnet/';

test('nested installation deep links reveal native OS tabs and focus the destination', async ({ page }) => {
  for (const id of ['python-on-windows', 'cli-on-windows', 'python-on-macos-or-linux']) {
    await page.goto(base + 'start/installation/#' + id);
    const target = page.locator('#' + id);
    await expect(target).toBeVisible();
    await expect(target).toBeInViewport();
    await expect(target).toBeFocused();
    await page.reload();
    await expect(target).toBeFocused();
    await expect(target).toBeInViewport();
    await page.keyboard.press('Tab');
    await expect.poll(() => page.evaluate(() => !!document.activeElement?.checkVisibility())).toBe(true);
  }
  await page.goto(base + 'start/installation/');
  const cli = page.getByRole('tab', { name: 'CLI', exact: true });
  const python = page.getByRole('tab', { name: 'Python', exact: true });
  await cli.click();
  await cli.press('ArrowRight');
  await expect(python).toBeFocused();
  await expect(python).toHaveAttribute('aria-selected', 'true');
  await python.press('End');
  await expect(page.getByRole('tab', { name: 'Rust', exact: true })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(cli).toBeFocused();
  await python.click();
  const unix = page.getByRole('tab', { name: 'macOS / Linux', exact: true });
  await unix.click();
  await unix.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Windows', exact: true })).toBeFocused();
  await expect(page.locator('#python-on-windows')).toBeVisible();
  await expect(cli).toHaveAttribute('aria-selected', 'false');
  for (const theme of ['light', 'dark'] as const) {
    await selectTheme(page, theme);
    expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
  }
});

test('native navigation and theme selection persist through reload and system changes', async ({ page }) => {
  await page.goto(base + 'start/local-lab/');
  const menu = page.getByRole('button', { name: 'Menu', exact: true });
  if (await menu.isVisible()) {
    await menu.click();
    const sidebar = page.locator('#starlight__sidebar');
    await expect(sidebar).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(sidebar).not.toBeVisible();
    await expect(menu).toBeFocused();
    await menu.press('Enter');
    await sidebar.getByRole('link', { name: 'Install the tools', exact: true }).click();
    await expect(page).toHaveURL(base + 'start/installation/');
    await expect(sidebar).not.toBeVisible();
  } else {
    await page.locator('#starlight__sidebar').getByRole('link', { name: 'Install the tools', exact: true }).click();
    await expect(page).toHaveURL(base + 'start/installation/');
  }
  await selectTheme(page, 'dark');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.goto(base);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await selectTheme(page, 'light');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await selectTheme(page, 'auto');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});

test('diagram keyboard zoom, panning, focus containment, close and reset', async ({ page, context }, info) => {
  await page.goto(base + 'start/local-lab/');
  const opener = page.getByRole('button', { name: /Expand diagram/ });
  await opener.click();
  const dialog = page.getByRole('dialog', { name: 'A complete read, on one machine' });
  const close = dialog.getByRole('button', { name: 'Close diagram' });
  await expect(close).toBeFocused();
  await page.keyboard.press('Tab');
  const slider = dialog.getByRole('slider', { name: 'Zoom' });
  await expect(slider).toBeFocused();
  await slider.press('End');
  await expect(slider).toHaveValue('300');
  await expect(dialog.locator('output')).toHaveText('300%');
  const viewport = dialog.getByRole('region', { name: 'Diagram; scroll to pan when zoomed' });
  await page.keyboard.press('Tab'); // Fit
  await page.keyboard.press('Tab'); // original SVG
  await page.keyboard.press('Tab'); // panning region
  await expect(viewport).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => viewport.evaluate(node => node.scrollLeft)).toBeGreaterThan(0);
  if (info.project.use.hasTouch) {
    const scrollBefore = await viewport.evaluate(node => node.scrollLeft);
    const pageBefore = await page.evaluate(() => scrollY);
    const area = (await viewport.boundingBox())!;
    const session = await context.newCDPSession(page);
    const x = area.x + area.width * 0.8;
    const y = area.y + Math.min(area.height / 2, 120);
    await session.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y }] });
    for (let step = 1; step <= 10; step++) {
      await session.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: x - step * 15, y }] });
    }
    await session.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await expect.poll(() => viewport.evaluate(node => node.scrollLeft)).toBeGreaterThan(scrollBefore);
    expect(await page.evaluate(() => scrollY)).toBe(pageBefore);
    await session.detach();
  }
  await expectContainedPage(page);
  const box = await dialog.boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(page.viewportSize()!.width);
  for (let index = 0; index < 12; index++) {
    await page.keyboard.press('Tab');
    expect(await page.evaluate(() => document.activeElement === document.body || !!document.activeElement?.closest('dialog[open]'))).toBe(true);
  }
  expect(await dialog.getAttribute('data-pagefind-ignore')).not.toBeNull();
  for (const theme of ['light', 'dark'] as const) {
    // Change theme through the native control outside the modal, then reopen.
    await close.click();
    await expect(opener).toBeFocused();
    await selectTheme(page, theme);
    await opener.click();
    expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
  }
  await close.click();
  await expect(opener).toBeFocused();
  await opener.press('Enter');
  await expect(slider).toHaveValue('100');
  await slider.press('End');
  await dialog.getByRole('button', { name: 'Fit', exact: true }).click();
  await expect(slider).toHaveValue('100');
  await expect.poll(() => viewport.evaluate(node => node.scrollLeft)).toBe(0);
  await page.keyboard.press('Escape');
  await expect(dialog).not.toBeVisible();
  await expect(opener).toBeFocused();
});

test('original diagram and text remain available without JavaScript', async ({ browser, baseURL }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, baseURL });
  try {
    const page = await context.newPage();
    await page.goto(base + 'start/local-lab/');
    await expect(page.locator('[data-diagram-open]')).toBeHidden();
    await expect(page.locator('rb-diagram figcaption')).toContainText('Analog input 1 returns a real value of 72.5');
    const link = page.getByRole('link', { name: /Original SVG/ });
    const popup = page.waitForEvent('popup');
    await link.click();
    const svg = await popup;
    await svg.waitForLoadState();
    expect(svg.url()).toContain(base + 'diagrams/local-lab.svg');
    await expect(svg.locator('svg')).toBeVisible();
  } finally {
    await context.close();
  }
});

test('native code copy writes the displayed code to the clipboard', async ({ page, context, browserName }) => {
  // Clipboard permission is a Chromium-only automation capability, not a UI substitute.
  expect(browserName).toBe('chromium');
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto(base + 'start/first-read/');
  const block = page.locator('.expressive-code').first();
  const code = await block.locator('code').innerText();
  await block.getByRole('button', { name: /Copy/ }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(code);
  await page.goto(base + 'start/local-lab/');
  await page.getByText('Show the complete loopback_lab.py script', { exact: true }).click();
  await page.locator('details .expressive-code').getByRole('button', { name: /Copy/ }).click();
  const source = await readFile(new URL('../../examples/python/loopback_lab.py', import.meta.url), 'utf8');
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(source.trimEnd());
});
