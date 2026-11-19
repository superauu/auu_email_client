import { expect, test } from '@playwright/test';

test('saved searches: save from the header, apply from the sidebar, delete persists', async ({ page }, info) => {
  await page.goto('/');
  const searchBox = page.getByRole('searchbox', { name: '搜索邮件' });
  await searchBox.fill('咖啡');
  // 有搜索词时出现保存入口；对话框预填当前搜索词。
  await page.getByRole('button', { name: '保存当前搜索' }).click();
  const dialog = page.getByRole('dialog', { name: '保存搜索' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('textbox', { name: '搜索名称' })).toHaveValue('咖啡');
  await dialog.getByRole('textbox', { name: '搜索名称' }).fill('咖啡邀请');
  await dialog.getByRole('button', { name: '保存', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: '已保存搜索「咖啡邀请」' })).toBeVisible();
  const savedNav = page.getByRole('navigation', { name: '已保存搜索' });
  await expect(savedNav.getByRole('button', { name: '咖啡邀请', exact: true })).toBeVisible();
  // 清空搜索后列表恢复全量；点击保存条目一键恢复搜索词与过滤结果。
  await searchBox.fill('');
  await expect(page.getByRole('button', { name: '阅读邮件：周末，一起去喝杯咖啡？', exact: true })).toBeVisible();
  await savedNav.getByRole('button', { name: '咖啡邀请', exact: true }).click();
  await expect(searchBox).toHaveValue('咖啡');
  await expect(savedNav.getByRole('button', { name: '咖啡邀请', exact: true })).toHaveAttribute('aria-current', 'page');
  // 删除条目：区收起且删除在 reload 后仍然生效（localStorage 持久化）。
  await savedNav.getByRole('button', { name: '删除已保存搜索 咖啡邀请' }).click();
  await expect(savedNav).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole('navigation', { name: '已保存搜索' })).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('saved-search.png') });
});
