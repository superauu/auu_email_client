import { expect, test } from '@playwright/test';

test('reader font size applies to plain text instantly and persists across reloads', async ({ page }, info) => {
  await page.goto('/');
  // 首封邮件（纯文本）。
  await page.getByRole('button', { name: /^阅读邮件：/ }).first().click();
  const body = page.locator('.message-body').first();
  const before = await body.evaluate(node => getComputedStyle(node).fontSize);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('tab', { name: '外观与皮肤' }).click();
  await page.getByRole('button', { name: '特大', exact: true }).click();
  await page.keyboard.press('Escape');
  // 纯文本正文经 CSS 变量即时生效，无需重开邮件。
  const after = await body.evaluate(node => getComputedStyle(node).fontSize);
  expect(after).not.toBe(before);
  expect(parseFloat(after)).toBeGreaterThan(parseFloat(before));
  // 持久化：reload 后仍是特大。
  await page.reload();
  const persisted = await page.locator('.message-body').first().evaluate(node => getComputedStyle(node).fontSize);
  expect(parseFloat(persisted)).toBe(parseFloat(after));
  await page.screenshot({ path: info.outputPath('reader-font.png') });
});
