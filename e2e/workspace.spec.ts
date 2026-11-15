import { expect, test } from '@playwright/test';

test('account, search and empty-state navigation', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /^收件箱\s*6$/ })).toBeVisible();
  await page.getByRole('button', { name: '独立邮箱', exact: true }).click();
  await expect(page.getByRole('heading', { name: /^收件箱\s*1$/ })).toBeVisible();
  await expect(page.getByRole('region', { name: '邮件列表' }).getByText('让下一版界面，多一点呼吸感')).toHaveCount(0);
  await page.getByRole('searchbox', { name: '搜索邮件' }).fill('不存在的邮件');
  await expect(page.getByText('没有找到相关邮件')).toBeVisible();
});

test('skin and mode persist, dialogs trap focus and return it', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '外观与皮肤', exact: true }).click();
  await page.getByRole('button', { name: '使用皮肤 Graphite · 石墨' }).click();
  await page.getByRole('button', { name: '深色', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-skin', 'graphite');
  await expect(page.locator('html')).toHaveAttribute('data-mode', 'dark');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '外观与皮肤', exact: true })).toBeFocused();
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-skin', 'graphite');
  await expect(page.locator('html')).toHaveAttribute('data-mode', 'dark');
});

test('a saved draft survives reload and stays in the selected sending account', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  await page.getByLabel('发件人', { exact: true }).selectOption('work');
  await page.getByLabel('收件人', { exact: true }).fill('friend@example.test');
  await page.getByLabel('主题', { exact: true }).fill('架构测试草稿');
  await page.getByRole('textbox', { name: '邮件正文' }).fill('保存到演示草稿。');
  await page.getByRole('button', { name: '保存草稿' }).click();
  await expect(page.getByRole('status').filter({ hasText: '草稿已自动保存' })).toBeVisible();
  await page.getByRole('button', { name: '关闭撰写并返回列表' }).click();
  await page.reload();
  await page.getByRole('button', { name: '工作邮箱', exact: false }).click();
  await page.getByRole('navigation', { name: '邮件文件夹' }).getByRole('button', { name: '草稿箱' }).click();
  await expect(page.getByRole('heading', { name: '架构测试草稿', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '编辑草稿' }).click();
  await expect(page.getByLabel('发件人', { exact: true })).toHaveValue('work');
  await expect(page.getByRole('textbox', { name: '邮件正文' })).toHaveText('保存到演示草稿。');
});

test('closing auto-saves the draft, and an explicit button discards it', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  await page.getByLabel('主题', { exact: true }).fill('关闭即保存');
  // 关闭（Escape）不再要求确认：草稿自动保存后撰写面收起。
  await page.keyboard.press('Escape');
  await expect(page.locator('.compose-pane')).toHaveCount(0);
  await page.reload();
  await page.getByRole('navigation', { name: '邮件文件夹' }).getByRole('button', { name: '草稿箱' }).click();
  await expect(page.getByRole('heading', { name: '关闭即保存', exact: true })).toBeVisible();
  // 只有显式「放弃草稿」按钮才删除草稿。
  await page.getByRole('button', { name: '编辑草稿' }).click();
  await expect(page.getByRole('button', { name: '放弃草稿', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '放弃草稿', exact: true }).click();
  await expect(page.locator('.compose-pane')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: '关闭即保存', exact: true })).toHaveCount(0);
});

test('autosave survives reload, and changing the sender moves a single draft', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  await page.getByLabel('主题', { exact: true }).fill('自动保存的草稿');
  await page.getByRole('textbox', { name: '邮件正文' }).fill('不点击保存也能恢复。');
  await expect(page.getByRole('status').filter({ hasText: '草稿已自动保存' })).toBeVisible();
  await page.getByLabel('发件人', { exact: true }).selectOption('work');
  await expect(page.getByRole('status').filter({ hasText: '草稿已自动保存' })).toBeVisible();
  await page.getByRole('button', { name: '关闭撰写并返回列表' }).click();
  await page.reload();
  await page.getByRole('navigation', { name: '邮件文件夹' }).getByRole('button', { name: '草稿箱' }).click();
  await expect(page.getByRole('button', { name: '阅读邮件：自动保存的草稿', exact: true })).toHaveCount(1);
  await page.getByRole('button', { name: '编辑草稿' }).click();
  await expect(page.getByLabel('发件人', { exact: true })).toHaveValue('work');
  await expect(page.getByRole('textbox', { name: '邮件正文' })).toHaveText('不点击保存也能恢复。');
});

test('a failed autosave preserves editable text and closing does not discard it silently', async ({ page }) => {
  await page.addInitScript(() => {
    const write = Storage.prototype.setItem;
    Storage.prototype.setItem = function(key, value) {
      if (key === 'auumail.demo.drafts.v1') throw new DOMException('Quota exceeded', 'QuotaExceededError');
      write.call(this, key, value);
    };
  });
  await page.goto('/');
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  await page.getByRole('textbox', { name: '邮件正文' }).fill('保存失败时也不能丢失的内容');
  await expect(page.getByRole('alert')).toContainText('草稿未能保存');
  await page.getByRole('button', { name: '关闭撰写并返回列表' }).click();
  await page.getByRole('button', { name: '保存并关闭' }).click();
  await expect(page.locator('.compose-pane')).toBeVisible();
  await expect(page.getByRole('textbox', { name: '邮件正文' })).toHaveText('保存失败时也不能丢失的内容');
  await page.getByRole('button', { name: '继续编辑' }).click();
  await page.getByRole('textbox', { name: '邮件正文' }).fill('继续修改，仍保留在编辑器中');
});

test('reply drafts preserve reply metadata and empty subjects when reopened', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '回复邮件', exact: true }).click();
  await expect(page.getByLabel('发件人', { exact: true })).toBeDisabled();
  await page.getByLabel('主题', { exact: true }).fill('');
  await page.getByRole('textbox', { name: '邮件正文' }).fill('回复草稿正文');
  await expect(page.getByRole('status').filter({ hasText: '草稿已自动保存' })).toBeVisible();
  await page.getByRole('button', { name: '关闭撰写并返回列表' }).click();
  await page.reload();
  await page.getByRole('navigation', { name: '邮件文件夹' }).getByRole('button', { name: '草稿箱' }).click();
  await page.getByRole('button', { name: '编辑草稿' }).click();
  await expect(page.getByRole('heading', { name: '回复邮件', exact: true })).toBeVisible();
  await expect(page.getByLabel('主题', { exact: true })).toHaveValue('');
  await expect(page.getByLabel('发件人', { exact: true })).toHaveValue('work');
});

test('sidebar width: drag adjusts, double-click resets, choice persists', async ({ page }) => {
  await page.goto('/');
  const sidebar = page.getByRole('complementary', { name: '邮箱导航' });
  const handle = page.getByRole('separator', { name: '调整导航栏宽度' });
  await expect(sidebar).toHaveCSS('width', '222px');
  const box = await handle.boundingBox();
  const y = box!.y + box!.height / 2;
  const centerX = box!.x + box!.width / 2;
  await page.mouse.move(centerX, y);
  await page.mouse.down();
  await page.mouse.move(centerX + 60, y, { steps: 4 });
  await page.mouse.up();
  await expect(sidebar).toHaveCSS('width', '282px');
  expect(await page.evaluate(() => window.getSelection()?.toString() ?? '')).toBe('');
  await handle.dblclick();
  await expect(sidebar).toHaveCSS('width', '222px');
  await page.mouse.move(centerX, y);
  await page.mouse.down();
  await page.mouse.move(centerX + 80, y, { steps: 4 });
  await page.mouse.up();
  await expect(sidebar).toHaveCSS('width', '302px');
  await page.reload();
  await expect(sidebar).toHaveCSS('width', '302px');
});

test('event completion, associated mail, and keyboard resizing', async ({ page }) => {
  await page.goto('/');
  const handle = page.getByRole('separator', { name: '调整邮件列表宽度' });
  await handle.focus();
  await page.keyboard.press('ArrowRight');
  await expect(handle).toHaveAttribute('aria-valuenow', '360');
  await page.keyboard.press('End');
  await expect.poll(async () => Number(await handle.getAttribute('aria-valuenow'))).toBeGreaterThan(600);
  await handle.dblclick();
  await expect(handle).toHaveAttribute('aria-valuenow', '340');
  await page.getByRole('button', { name: '事件与待办' }).click();
  await page.getByRole('button', { name: '完成：阅读体验 · 设计讨论', exact: true }).click();
  await expect(page.getByRole('button', { name: '标记未完成：阅读体验 · 设计讨论', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: '查看邮件' }).first().click();
  await expect(page.getByRole('heading', { name: '让下一版界面，多一点呼吸感', exact: true })).toBeVisible();
});

test('narrow desktop navigation remains usable without horizontal overflow', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await page.goto('/');
  await page.getByRole('button', { name: '阅读邮件：让下一版界面，多一点呼吸感', exact: true }).click();
  await expect(page.getByRole('heading', { name: '让下一版界面，多一点呼吸感', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '返回邮件列表' }).click();
  await expect(page.getByRole('region', { name: '邮件列表' })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
});


test('compact Outlook-style list fits more mail and unread filtering remains scoped',async ({page},info)=>{
  await page.goto('/');const list=page.getByRole('region',{name:'邮件列表'});await expect(list.getByRole('button',{name:'全部',exact:true})).toHaveAttribute('aria-pressed','true');
  await expect(list.getByText('YOUR MAIL, IN ORDER')).toHaveCount(0);await expect(list.getByText('所有账号的邮件，尽在一处')).toHaveCount(0);
  const bounds=await list.locator('article').evaluateAll(rows=>rows.map(row=>row.getBoundingClientRect().height));expect(bounds.length).toBe(6);expect(bounds.every(height=>height<=81)).toBeTruthy();
  await list.getByRole('button',{name:'未读',exact:true}).click();await expect(list.locator('article')).toHaveCount(3);
  await page.getByRole('button',{name:'独立邮箱',exact:true}).click();await expect(list.getByRole('heading',{name:'没有未读邮件',exact:true})).toBeVisible();
  await list.getByRole('button',{name:'全部',exact:true}).click();await expect(list.locator('article')).toHaveCount(1);
  await page.getByRole('button',{name:/^全部账号/}).click();await page.screenshot({path:info.outputPath('compact-mail-list.png')});
});


test('reading an unread result keeps that message open after marking read and account changes isolate it',async ({page})=>{
  await page.goto('/');const list=page.getByRole('region',{name:'邮件列表'});await list.getByRole('button',{name:'未读',exact:true}).click();
  await list.getByRole('button',{name:'阅读邮件：让下一版界面，多一点呼吸感',exact:true}).click();
  await expect(list.locator('article')).toHaveCount(2);await expect(page.getByRole('heading',{name:'让下一版界面，多一点呼吸感',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'独立邮箱',exact:true}).click();await expect(page.getByRole('heading',{name:'让下一版界面，多一点呼吸感',exact:true})).toHaveCount(0);
});


test('aligned toolbars and wide list resizing preserve reader space and stored preference',async ({page},info)=>{
  await page.setViewportSize({width:1920,height:1080});await page.goto('/');
  const list=page.getByRole('region',{name:'邮件列表'});const handle=page.getByRole('separator',{name:'调整邮件列表宽度'});
  await expect(list.getByRole('button',{name:'全部',exact:true})).toBeVisible();
  const bars=await page.locator('.list-toolbar, .reader-toolbar').evaluateAll(bars=>bars.map(bar=>{const r=bar.getBoundingClientRect();return {top:r.top,bottom:r.bottom,height:r.height};}));
  expect(bars).toHaveLength(2);expect(bars[0]).toEqual(bars[1]);expect(bars[0].height).toBe(44);
  await handle.focus();await page.keyboard.press('End');await expect(list).toHaveCSS('width','900px');
  await page.reload();await expect(list).toHaveCSS('width','900px');
  await page.setViewportSize({width:1100,height:800});await expect.poll(async()=>Number(await handle.getAttribute('aria-valuemax'))).toBeLessThan(900);
  const reader=page.locator('.mail-reader');expect((await reader.boundingBox())!.width).toBeGreaterThanOrEqual(320);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
  await page.setViewportSize({width:1920,height:1080});await expect(list).toHaveCSS('width','900px');await page.screenshot({path:info.outputPath('wide-list-aligned-toolbars.png')});
});

test('demo event completion persists across reloads', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '事件与待办', exact: true }).click();
  await page.getByRole('button', { name: '完成：阅读体验 · 设计讨论', exact: true }).click();
  await page.reload();
  await page.getByRole('button', { name: '事件与待办', exact: true }).click();
  await expect(page.getByRole('button', { name: '标记未完成：阅读体验 · 设计讨论', exact: true })).toBeVisible();
});
