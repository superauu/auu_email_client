import { expect, test } from '@playwright/test';

test('common shortcuts and collapsible account directories isolate accounts and keep pins across reload', async ({ page }) => {
  await page.goto('/');
  const common = page.getByRole('navigation', { name: '常用入口', exact: true });
  await expect(common.getByRole('button', { name: '全部收件箱', exact: true })).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('navigation', { name: '服务器文件夹', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: '工作邮箱', exact: true }).click();
  const folders = page.getByRole('navigation', { name: '工作邮箱的文件夹', exact: true });
  await expect(folders).toBeVisible();
  await expect(page.getByRole('heading', { name: /^收件箱\s*2$/ })).toBeVisible();
  const inbox = folders.getByRole('button', { name: '收件箱', exact: true });
  await inbox.hover();
  await folders.getByRole('button', { name: '固定 工作邮箱 / 收件箱', exact: true }).click();
  const shortcut = common.getByRole('button', { name: '收件箱 · 工作邮箱', exact: true });
  await expect(shortcut).toBeVisible();
  await page.getByRole('button', { name: '折叠工作邮箱文件夹' }).click();
  await expect(folders).toBeHidden();
  await common.getByRole('button', { name: '全部收件箱', exact: true }).click();
  await expect(page.getByRole('heading', { name: /^全部收件箱\s*6$/ })).toBeVisible();
  await page.reload();
  await expect(shortcut).toBeVisible();
  await expect(folders).toBeHidden();
  await shortcut.click();
  await expect(folders).toBeVisible();
  await expect(page.getByRole('heading', { name: /^收件箱\s*2$/ })).toBeVisible();
  await expect(page.getByRole('button', { name: '阅读邮件：周末，一起去喝杯咖啡？', exact: true })).toHaveCount(0);
  await common.getByRole('button', { name: '取消固定 工作邮箱 / 收件箱', exact: true }).click();
  await expect(shortcut).toHaveCount(0);
  await expect(inbox).toHaveAttribute('aria-current', 'page');
  await page.reload(); await expect(shortcut).toHaveCount(0);
});

test('account expand buttons work with keyboard and narrow layout remains usable', async ({ page }, info) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await page.goto('/');
  const expand = page.getByRole('button', { name: '展开工作邮箱文件夹', exact: true });
  await expand.focus(); await page.keyboard.press('Enter');
  const folders = page.getByRole('navigation', { name: '工作邮箱的文件夹', exact: true });
  await expect(folders).toBeVisible();
  await folders.getByRole('button', { name: '本机草稿', exact: true }).click();
  await expect(page.getByRole('heading', { name: /^本机草稿\s*0$/ })).toBeVisible();
  await expect(page.getByRole('region', { name: '邮件列表' })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.screenshot({ path: info.outputPath('navigation-narrow.png') });
});

for (const skinId of ['paper', 'graphite', 'linen']) for (const mode of ['light', 'dark']) {
  test(`navigation appearance ${skinId} ${mode}`, async ({ page }, info) => {
    await page.addInitScript(({ skinId, mode }) => localStorage.setItem('auumail.ui.theme.v1', JSON.stringify({ skinId, mode })), { skinId, mode });
    await page.goto('/'); await page.getByRole('button', { name: '工作邮箱', exact: true }).click();
    await page.getByRole('button', { name: '折叠个人邮箱文件夹', exact: true }).click();
    await page.getByRole('navigation', { name: '工作邮箱的文件夹' }).getByRole('button', { name: '固定 工作邮箱 / 收件箱', exact: true }).click();
    await expect(page.getByRole('navigation', { name: '常用入口' }).getByRole('button', { name: '收件箱 · 工作邮箱', exact: true })).toBeVisible();
    await page.screenshot({ path: info.outputPath(`navigation-${skinId}-${mode}.png`) });
  });
}
