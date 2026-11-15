import { expect, test } from '@playwright/test';

test('recipient chips, cc and bcc survive closing, sender changes and reload', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  const pane = page.locator('.compose-pane');
  const to = pane.getByRole('combobox', { name: '收件人', exact: true });
  await to.fill('first@example.test; second@example.test;');
  await expect(pane.locator('.recipient-chip')).toHaveCount(2);
  await pane.getByRole('button', { name: '抄送', exact: true }).click();
  await pane.getByRole('combobox', { name: '抄送', exact: true }).fill('copy@example.test');
  await pane.getByRole('button', { name: '密送', exact: true }).click();
  await pane.getByRole('combobox', { name: '密送', exact: true }).fill('private@example.test');
  await pane.getByLabel('发件人', { exact: true }).selectOption('work');
  await pane.getByLabel('主题', { exact: true }).fill('多人收件草稿');
  await pane.getByRole('button', { name: '从收件人移除 first@example.test', exact: true }).click();
  await pane.getByRole('button', { name: '关闭撰写并返回列表' }).click();
  await page.reload();
  await page.getByRole('navigation', { name: '常用入口' }).getByRole('button', { name: '本机草稿', exact: true }).click();
  await page.getByRole('button', { name: '编辑草稿' }).click();
  await expect(pane.getByLabel('发件人', { exact: true })).toHaveValue('work');
  for (const [field, email] of [['收件人', 'second'], ['抄送', 'copy'], ['密送', 'private']]) await expect(pane.getByRole('button', { name: `从${field}移除 ${email}@example.test`, exact: true })).toBeVisible();
  await expect(pane.getByRole('button', { name: '从收件人移除 first@example.test', exact: true })).toHaveCount(0);
});

test('invalid addresses remain editable with keyboard and local suggestions dismiss before composer', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  const to = page.getByRole('combobox', { name: '收件人', exact: true });
  await to.fill('invalid'); await to.press('Enter');
  await expect(to).toHaveAttribute('aria-invalid', 'true');
  await to.press('Backspace'); await expect(to).toHaveValue('invalid');
  await to.fill('valid@example.test'); await to.press('Enter');
  await expect(to).toHaveAttribute('aria-invalid', 'false');
  await page.getByLabel('发件人', { exact: true }).selectOption('work');
  await to.fill('zhixia');
  await expect(page.getByRole('listbox', { name: '收件人建议' })).toBeVisible();
  await to.press('Escape'); await expect(page.locator('.compose-pane')).toBeVisible();
  await expect(page.getByRole('listbox')).toHaveCount(0);
  await to.fill('zhix'); await to.press('ArrowDown'); await to.press('Enter');
  await expect(page.getByRole('button', { name: '从收件人移除 zhixia@example.test', exact: true })).toBeVisible();
});

test('legacy plain text restores without modifying it and a blank composer creates no draft', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('auumail.demo.drafts.v1', JSON.stringify([{ id: 'draft-legacy', accountId: 'personal', to: '', subject: '旧草稿', bodyText: '第一行\n第二行 <保持原文>', updatedAt: '2026-11-15T00:00:00Z', revision: 1 }])));
  await page.goto('/');
  await page.getByRole('navigation', { name: '常用入口' }).getByRole('button', { name: '本机草稿', exact: true }).click();
  await page.getByRole('button', { name: '编辑草稿' }).click();
  await expect(page.getByRole('textbox', { name: '邮件正文' })).toHaveText('第一行第二行 <保持原文>');
  await page.getByRole('button', { name: '关闭撰写并返回列表' }).click();
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  await page.getByRole('button', { name: '关闭撰写并返回列表' }).click();
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('auumail.demo.drafts.v1')!));
  expect(stored).toHaveLength(1); expect(stored[0].bodyText).toBe('第一行\n第二行 <保持原文>'); expect(stored[0].revision).toBe(1);
});

test('formatting works with keyboard and link insertion preserves the selected text', async ({ page }) => {
  await page.goto('/'); await page.getByRole('button', { name: '写邮件', exact: false }).click();
  const editor = page.getByRole('textbox', { name: '邮件正文' });
  await editor.fill('需要强调的正文'); await editor.press('Control+A');
  const bold = page.getByRole('button', { name: '加粗 (Ctrl+B)', exact: true });
  await bold.focus(); await bold.press('Enter');
  await expect(editor.locator('b,strong')).toHaveText('需要强调的正文');
  await page.getByRole('button', { name: '插入链接', exact: true }).click();
  await page.getByLabel('链接地址', { exact: true }).fill('https://example.test/review');
  await page.getByRole('button', { name: '插入', exact: true }).click();
  await expect(editor.locator('a')).toHaveAttribute('href', 'https://example.test/review');
  await expect(editor.locator('a')).toHaveText('需要强调的正文');
  await page.getByRole('button', { name: '插入链接', exact: true }).click();
  await page.getByLabel('链接地址', { exact: true }).press('Escape');
  await expect(page.locator('.compose-pane')).toBeVisible();
});

test('paste keeps safe formatting while dropping remote images and app-level styles', async ({ page }) => {
  await page.goto('/'); await page.getByRole('button', { name: '写邮件', exact: false }).click();
  const editor = page.getByRole('textbox', { name: '邮件正文' }); await editor.click();
  await editor.evaluate(element => {
    const clipboardData = new DataTransfer();
    clipboardData.setData('text/html', '<style>body{display:none}</style><p class="app-header" onclick="alert(1)" style="background-image:url(https://example.test/tracker)"><b>粘贴的内容</b><img src="https://example.test/tracker"><a href="javascript:alert(1)">链接</a></p>');
    element.dispatchEvent(new ClipboardEvent('paste', { clipboardData, bubbles: true, cancelable: true }));
  });
  await expect(editor.locator('b,strong')).toHaveText('粘贴的内容');
  await expect(editor.locator('img, style, [class], [onclick], [href]')).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: '图片未插入' })).toBeVisible();
});

for (const skin of ['paper', 'graphite', 'linen']) for (const mode of ['light', 'dark']) {
  test(`compose layout ${skin} ${mode} supports long recipients and narrow windows`, async ({ page }, info) => {
    await page.addInitScript(({ skinId, mode }) => localStorage.setItem('auumail.ui.theme.v1', JSON.stringify({ skinId, mode })), { skinId: skin, mode });
    await page.goto('/'); await page.getByRole('button', { name: '写邮件', exact: false }).click();
    await page.getByLabel('主题', { exact: true }).fill('项目方案评审 · 请确认本周交付安排');
    const to = page.getByRole('combobox', { name: '收件人', exact: true });
    await to.fill('design-team@example.test; a-very-long-recipient-name-for-layout@example.test;');
    await page.getByRole('textbox', { name: '邮件正文' }).fill('各位同事，\n\n本周的方案已整理完成，请查看并反馈修改建议。\n\n谢谢！');
    await page.screenshot({ path: info.outputPath(`${skin}-${mode}-wide.png`) });
    await page.getByRole('button', { name: '展开撰写窗格' }).click();
    await expect(page.getByRole('region', { name: '邮件列表' })).toBeHidden();
    await expect(page.getByRole('textbox', { name: '邮件正文' })).toContainText('本周的方案');
    await page.getByRole('button', { name: '还原撰写窗格' }).click();
    await page.setViewportSize({ width: 900, height: 700 });
    await expect(page.getByRole('textbox', { name: '邮件正文' })).toBeVisible();
    await expect(page.getByRole('button', { name: '放弃草稿', exact: true })).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
    await page.screenshot({ path: info.outputPath(`${skin}-${mode}-narrow.png`) });
  });
}

test('format toolbar stays above the editor and Enter adds one compact line', async ({ page }) => {
  await page.goto('/'); await page.getByRole('button', { name: '写邮件', exact: false }).click();
  const toolbar = page.getByRole('toolbar', { name: '格式工具栏' });
  const editor = page.getByRole('textbox', { name: '邮件正文' });
  await editor.fill('第一行'); await editor.press('End'); await editor.press('Enter'); await editor.pressSequentially('第二行');
  const metrics = await editor.evaluate(element => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    const tops: number[] = []; let node;
    while ((node = walker.nextNode())) if (node.textContent?.trim()) { const range = document.createRange(); range.selectNodeContents(node); tops.push(range.getBoundingClientRect().top); }
    return { tops, lineHeight: parseFloat(getComputedStyle(element).lineHeight) };
  });
  expect(metrics.tops).toHaveLength(2);
  expect(metrics.tops[1] - metrics.tops[0]).toBeCloseTo(metrics.lineHeight, 0);
  expect((await toolbar.boundingBox())!.y + (await toolbar.boundingBox())!.height).toBeLessThanOrEqual((await editor.boundingBox())!.y);
  await editor.press('Control+Home'); await editor.press('Shift+Enter');
  await expect(editor).toContainText('第一行');
});

test('newsletter quotation keeps borderless tables, spacing and colors after draft recovery', async ({ page }, info) => {
  const bodyHtml = '<p><br></p><div>原始邮件</div><blockquote data-auumail-quote="true"><div style="color:#262626;background-color:#ffffff;font:14px Arial;line-height:20px"><table width="620" border="0" cellspacing="0" cellpadding="0" align="center"><tr><td><table width="100%" border="0" cellspacing="0" cellpadding="0"><tr><td style="padding:24px;text-align:center"><h1 style="font-size:24px;line-height:30px;margin:0">产品动态</h1><p style="margin:8px 0 0">这里是原邮件的内容。</p></td></tr><tr><td bgcolor="#f4f4f4" style="padding:16px;text-align:center"><a href="https://example.test/review" style="color:#ffffff;background-color:#4545cc;display:inline-block;padding:8px 16px;text-decoration:none">查看详情</a></td></tr></table></td></tr></table></div></blockquote>';
  await page.addInitScript(({ bodyHtml }) => {
    localStorage.setItem('auumail.ui.theme.v1', JSON.stringify({ skinId: 'paper', mode: 'dark' }));
    localStorage.setItem('auumail.demo.drafts.v1', JSON.stringify([{ id: 'draft-newsletter', accountId: 'personal', to: 'team@example.test', subject: '回复排版回归', bodyText: '原始邮件\n产品动态', bodyHtml, updatedAt: '2026-11-15T00:00:00Z', revision: 1 }]));
  }, { bodyHtml });
  await page.goto('/');
  await page.getByRole('navigation', { name: '常用入口' }).getByRole('button', { name: '本机草稿', exact: true }).click();
  await page.getByRole('button', { name: '编辑草稿' }).click();
  const editor = page.getByRole('textbox', { name: '邮件正文' });
  const cells = editor.locator('td');
  await expect(cells.first()).toHaveCSS('padding-top', '0px');
  await expect(cells.first()).toHaveCSS('border-top-width', '0px');
  await expect(cells.nth(1)).toHaveCSS('padding-top', '24px');
  await expect(editor.locator('table').first()).toHaveCSS('width', '620px');
  await expect(editor.getByRole('heading', { name: '产品动态' })).toHaveCSS('font-size', '24px');
  await expect(editor.locator('blockquote > div')).toHaveCSS('color', 'rgb(38, 38, 38)');
  await expect(editor.getByRole('link')).toHaveCSS('color', 'rgb(255, 255, 255)');
  await editor.press('Control+Home'); await editor.pressSequentially('保留引用，继续回复');
  await page.getByRole('button', { name: '关闭撰写并返回列表' }).click();
  await page.getByRole('button', { name: '编辑草稿' }).click();
  await expect(editor.locator('td').nth(1)).toHaveCSS('padding-top', '24px');
  await page.setViewportSize({ width: 900, height: 700 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.screenshot({ path: info.outputPath('newsletter-quote-fixed.png') });
});
