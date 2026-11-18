import test from 'node:test';
import assert from 'node:assert/strict';
import { conversationRows, groupBySubject, normalizeSubject } from '../src/features/mail/threading';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { MailMessage } from '../src/domain/mail';

const base = demoWorkspace.messages[0];
const message = (overrides: Partial<MailMessage>): MailMessage => ({
  ...base, id: overrides.id ?? 'm', labelIds: [], starred: false, unread: false, ...overrides,
});

test('normalizeSubject strips reply and forward prefixes in Chinese and Latin, repeatedly', () => {
  assert.equal(normalizeSubject('Re: 季度评审'), '季度评审');
  assert.equal(normalizeSubject('RE: 季度评审'), '季度评审');
  assert.equal(normalizeSubject('Re[2]: 季度评审'), '季度评审');
  assert.equal(normalizeSubject('Fwd: 季度评审'), '季度评审');
  assert.equal(normalizeSubject('FW : 季度评审'), '季度评审');
  assert.equal(normalizeSubject('回复： 季度评审'), '季度评审');
  assert.equal(normalizeSubject('转发:季度评审'), '季度评审');
  assert.equal(normalizeSubject('答复：季度评审'), '季度评审');
  // 多轮前缀逐层剥离。
  assert.equal(normalizeSubject('Re: Fwd: 回复：季度评审'), '季度评审');
  // 无前缀原样归一（折叠空白 + 小写）。
  assert.equal(normalizeSubject('  Multiple   Spaces HERE '), 'multiple spaces here');
  // 剥完为空或纯前缀不折叠。
  assert.equal(normalizeSubject('Re:'), '');
  assert.equal(normalizeSubject('   '), '');
  assert.equal(normalizeSubject('季度评审'), '季度评审');
});

test('groupBySubject groups only two or more same-subject messages within one account', () => {
  const messages: MailMessage[] = [
    message({ id: 'a3', accountId: 'a', subject: 'Re: 方案评审', receivedAt: '2026-01-03T00:00:00Z' }),
    message({ id: 'b1', accountId: 'b', subject: '方案评审', receivedAt: '2026-01-04T00:00:00Z' }),
    message({ id: 'a2', accountId: 'a', subject: '转发：方案评审', receivedAt: '2026-01-02T00:00:00Z', unread: true }),
    message({ id: 'a1', accountId: 'a', subject: '方案评审', receivedAt: '2026-01-01T00:00:00Z' }),
    message({ id: 'solo', accountId: 'a', subject: '独立邮件', receivedAt: '2026-01-05T00:00:00Z' }),
    message({ id: 'empty', accountId: 'a', subject: 'Re:', receivedAt: '2026-01-06T00:00:00Z' }),
  ];
  const groups = groupBySubject(messages);
  // 账号 b 的同名单封不成组；跨账号不混。
  assert.equal(groups.length, 1);
  assert.equal(groups[0].key, 'a\u{1f}方案评审');
  // 组内时间正序，展示主题取会话根（最早一封）。
  assert.deepEqual(groups[0].messages.map(item => item.id), ['a1', 'a2', 'a3']);
  assert.equal(groups[0].subject, '方案评审');
});

test('conversationRows folds groups at the newest message position and expands chronologically', () => {
  const messages: MailMessage[] = [
    message({ id: 'new', accountId: 'a', subject: 'Re: 方案评审', receivedAt: '2026-01-03T00:00:00Z', unread: true }),
    message({ id: 'solo', accountId: 'a', subject: '独立邮件', receivedAt: '2026-01-02T12:00:00Z' }),
    message({ id: 'mid', accountId: 'a', subject: '转发：方案评审', receivedAt: '2026-01-02T00:00:00Z' }),
    message({ id: 'old', accountId: 'a', subject: '方案评审', receivedAt: '2026-01-01T00:00:00Z', unread: true }),
  ];
  const collapsed = conversationRows(messages, new Set());
  // 折叠：组行出现在组内最新一封的位置（列表首位），普通行保持骨架；组行带未读计数。
  assert.deepEqual(collapsed.map(row => row.kind), ['group', 'message']);
  const groupRow = collapsed[0];
  assert(groupRow.kind === 'group');
  assert.equal(groupRow.latest.id, 'new');
  assert.equal(groupRow.unread, 2);
  assert.equal(groupRow.group.messages.length, 3);
  assert.equal(groupRow.group.subject, '方案评审');

  const expanded = conversationRows(messages, new Set([groupRow.group.key]));
  // 展开：组行保留（作为折叠入口，aria-expanded），组内时间正序（旧 → 新），子行标注 childOf。
  assert.deepEqual(expanded.map(row => row.kind), ['group', 'message', 'message', 'message', 'message']);
  assert.equal(expanded[0].kind === 'group' && expanded[0].expanded, true);
  assert.deepEqual(expanded.slice(1, 4).map(row => (row.kind === 'message' ? row.message.id : '')), ['old', 'mid', 'new']);
  assert.ok(expanded.slice(1, 4).every(row => row.kind === 'message' && row.childOf === groupRow.group.key));
  assert.equal(expanded[4].kind === 'message' && expanded[4].childOf, undefined);
});

test('conversationRows keeps every message row untouched when nothing shares a subject', () => {
  const messages: MailMessage[] = [
    message({ id: 'one', accountId: 'a', subject: '唯一甲' }),
    message({ id: 'two', accountId: 'a', subject: '唯一乙' }),
  ];
  const rows = conversationRows(messages, new Set());
  assert.deepEqual(rows.map(row => (row.kind === 'message' ? row.message.id : '')), ['one', 'two']);
});
