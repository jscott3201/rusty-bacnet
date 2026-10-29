import { test, expect } from '@playwright/test';
const base = '/rusty-bacnet/';

const pages = [
  ['Reach devices on other subnets', 'connect/across-subnets/', false],
  ['BACnet/IP across NAT', 'connect/nat/', false],
  ['Join a B/IP multicast group', 'connect/multicast/', false],
  ['Connect over BACnet/IPv6', 'connect/ipv6/', false],
] as const;

test('the overview links each networking page, which keeps dev additions in its own section', async ({ page, request }) => {
  for (const [title, path, hasDev] of pages) {
    await page.goto(base + 'connect/overview/');
    await page.locator('main').getByRole('link', { name: title, exact: true }).first().click();
    await expect(page).toHaveURL(base + path);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(title);
    const main = page.locator('.sl-markdown-content');
    await expect(main.getByRole('heading', { level: 2, name: 'Next steps' })).toBeVisible();
    if (hasDev) {
      await main.getByRole('link', { name: /its own section|their own section/ }).click();
      await expect(page).toHaveURL(base + path + '#on-dev-after-the-release');
    } else {
      await expect(main.locator('#on-dev-after-the-release')).toHaveCount(0);
    }
    const raw = await request.get(base + 'raw/' + path.replace(/\/$/, '.md'));
    expect(raw.status()).toBe(200);
    expect(await raw.text()).toContain('## Next steps');
  }
});
