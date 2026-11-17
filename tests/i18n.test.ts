import test from 'node:test';
import assert from 'node:assert/strict';
import { zhCN } from '../src/i18n/messages/zh-CN';
import { enUS } from '../src/i18n/messages/en-US';
import { t, setLanguage, initializeLanguage, currentLanguage, languageLabel, languages, isLanguage, subscribeI18n } from '../src/i18n/core';

test('english catalog covers every zh-CN message id one-to-one', () => {
  const zhKeys = Object.keys(zhCN);
  const enKeys = Object.keys(enUS);
  assert.equal(zhKeys.length, enKeys.length, '字典键数量必须一致');
  for (const key of zhKeys) {
    assert.ok(enKeys.includes(key), `en 缺少键 ${key}`);
    assert.ok(typeof enUS[key as keyof typeof enUS] === 'string' && enUS[key as keyof typeof enUS].length > 0, `en 键 ${key} 为空`);
  }
  // 插值占位符集合一致：翻译不得丢失或新增参数。
  for (const key of zhKeys) {
    const zhParams = [...(zhCN[key as keyof typeof zhCN] as string).matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort();
    const enParams = [...enUS[key as keyof typeof enUS].matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort();
    assert.deepEqual(enParams, zhParams, `键 ${key} 的插值参数不一致`);
  }
});

test('t interpolates params and keeps unknown placeholders verbatim', () => {
  initializeLanguage('zh-CN');
  assert.equal(t('app.statusAccounts', { count: 3 }), '3 个邮箱账号');
  assert.equal(t('sidebar.draftCount', { count: 0 }), '0 份草稿');
  assert.ok(t('settings.deleteFailed', { name: 'A', error: 'X' }).includes('A'));
  assert.ok(t('settings.deleteFailed', { name: 'A', error: 'X' }).includes('X'));
  // 缺参时保留占位符原文，不产生 undefined。
  const partial = t('settings.lastSync', {});
  assert.ok(partial.includes('{time}'), '缺失参数应保留占位符');
});

test('setLanguage switches catalog output and notifies subscribers', () => {
  initializeLanguage('zh-CN');
  assert.equal(t('compose.send'), '发送');
  let notified = 0;
  const unsubscribe = subscribeI18n(() => { notified += 1; });
  setLanguage('en-US');
  assert.equal(currentLanguage(), 'en-US');
  assert.equal(t('compose.send'), 'Send');
  assert.equal(notified, 1);
  // 同语言重复 set 不通知。
  setLanguage('en-US');
  assert.equal(notified, 1);
  unsubscribe();
  setLanguage('zh-CN');
  // 退订后不再通知；语言输出仍恢复中文。
  assert.equal(notified, 1);
  assert.equal(t('compose.send'), '发送');
});

test('initializeLanguage and helpers behave', () => {
  initializeLanguage('en-US');
  assert.equal(currentLanguage(), 'en-US');
  assert.equal(t('settings.tabAbout'), 'About');
  assert.deepEqual([...languages], ['zh-CN', 'en-US']);
  assert.equal(isLanguage('zh-CN'), true);
  assert.equal(isLanguage('fr'), false);
  assert.equal(languageLabel('zh-CN'), '中文');
  assert.equal(languageLabel('en-US'), 'English');
  initializeLanguage('zh-CN');
});
