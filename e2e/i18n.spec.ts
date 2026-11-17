import { expect, test } from '@playwright/test';

test('language switch applies instantly, persists across reload and restores html lang', async ({ page }, info) => {
  await page.goto('/');
  // 测试 locale 为 zh-CN：默认中文界面。
  await expect(page.getByRole('button', { name: '写邮件', exact: false })).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');

  // 设置 → 外观与皮肤 → 界面语言 → English。
  await page.getByRole('button', { name: '外观与皮肤', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: '界面语言' })).toBeVisible();
  await dialog.getByRole('button', { name: 'English', exact: true }).click();

  // 即时生效：对话框文案与 html lang 切换为英文（模态下背景 aria-hidden，仅核对对话框内）。
  await expect(dialog.getByRole('heading', { name: 'Language', exact: true })).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('lang', 'en-US');

  // 关闭对话框后全局为英文；持久化：刷新后仍是英文。
  await dialog.getByRole('button', { name: 'Close dialog' }).click();
  await expect(page.getByRole('button', { name: 'New mail', exact: false })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('button', { name: 'New mail', exact: false })).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('lang', 'en-US');

  // 切回中文并刷新确认。
  await page.getByRole('button', { name: 'Appearance and skins', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: '中文', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: '关闭对话框' }).click();
  await expect(page.getByRole('button', { name: '写邮件', exact: false })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('button', { name: '写邮件', exact: false })).toBeVisible();
  await page.screenshot({ path: info.outputPath('i18n-back-to-zh.png') });
});

test('english workspace renders mail list and reading pane in English', async ({ page }, info) => {
  await page.addInitScript(() => localStorage.setItem('auumail.ui.lang.v1', JSON.stringify('en-US')));
  await page.goto('/');
  await expect(page.getByRole('region', { name: 'Mail list', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: /^All inboxes/ })).toBeVisible();
  await expect(page.getByRole('button', { name: 'New mail', exact: false })).toBeVisible();
  // 阅读一封邮件，验证阅读窗格与操作按钮为英文（演示邮件内容本身保持原语言）。
  await page.getByRole('button', { name: /Read mail:/ }).first().click();
  await expect(page.getByRole('button', { name: 'Reply', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Forward', exact: true })).toBeVisible();
  await page.screenshot({ path: info.outputPath('i18n-english-workspace.png') });
});
