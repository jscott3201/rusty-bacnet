import { expect, type Locator, type Page } from '@playwright/test';
import { sampleManifests } from '../../scripts/content.mjs';
import { noteParts, sampleNote } from '../../src/lib/samples.mjs';

/** A sample's note (src/lib/samples.mjs, #2648) is shown once in `main`, with its links and commands. */
export async function expectSampleNote(main: Locator, name: string) {
  for (const part of noteParts(sampleNote(name, await sampleManifests()))) {
    if ('block' in part) {
      const block = main.locator('pre code').filter({ hasText: part.block });
      await expect(block).toHaveCount(1);
      await expect(block).toBeVisible();
      continue;
    }
    const note = main.locator('p').filter({ hasText: part.inline.map((run) => run.text ?? run.code ?? run.strong).join('') });
    await expect(note).toHaveCount(1);
    await expect(note).toBeVisible();
    for (const run of part.inline.filter((run) => run.href)) {
      await expect(note.getByRole('link', { name: run.text, exact: true })).toHaveAttribute('href', run.href!);
    }
  }
}

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
