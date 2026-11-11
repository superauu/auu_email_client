import { expect, test } from '@playwright/test';

test('read, star and archive persist through offline reloads', async ({ page, context }, info) => {
  await page.goto('/');
  const title = '让下一版界面，多一点呼吸感';
  await page.getByRole('button', { name: `阅读邮件：${title}`, exact: true }).click();
  await expect(page.getByRole('button', { name: '标为未读', exact: true })).toBeEnabled();
  const star = page.getByRole('button', { name: '切换当前邮件星标', exact: true });
  const initiallyStarred = await star.getAttribute('aria-pressed');
  await star.click(); await expect(star).toHaveAttribute('aria-pressed', initiallyStarred === 'true' ? 'false' : 'true');
  await page.getByRole('button', { name: '标为未读', exact: true }).click();
  await expect(page.getByRole('button', { name: '标为已读', exact: true })).toBeEnabled();
  await page.reload();
  await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
  await expect(star).toHaveAttribute('aria-pressed', initiallyStarred === 'true' ? 'false' : 'true');
  await expect(page.getByRole('button', { name: '标为已读', exact: true })).toBeEnabled();
  await context.setOffline(true);
  await page.getByRole('button', { name: '归档当前邮件', exact: true }).click();
  await expect(page.getByRole('button', { name: `阅读邮件：${title}`, exact: true })).toHaveCount(0);
  await page.getByRole('navigation', { name: '邮件文件夹' }).getByRole('button', { name: '归档' }).click();
  await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: '已保存在本机' })).toBeVisible();
  await page.screenshot({ path: info.outputPath('offline-archive.png') });
  await context.setOffline(false); await page.reload();
  await page.getByRole('navigation', { name: '邮件文件夹' }).getByRole('button', { name: '归档' }).click();
  await expect(page.getByRole('button', { name: `阅读邮件：${title}`, exact: true })).toBeVisible();
});

test('storage failure keeps the message in place and explains that the operation failed', async ({ page }) => {
  await page.goto('/');
  const title = '让下一版界面，多一点呼吸感';
  await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
  await page.evaluate(() => { Storage.prototype.setItem = () => { throw new Error('Test storage unavailable'); }; });
  await page.getByRole('button', { name: '归档当前邮件', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('邮件状态未保存');
  await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `阅读邮件：${title}`, exact: true })).toBeVisible();
});
