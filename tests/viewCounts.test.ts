import test from 'node:test';
import assert from 'node:assert/strict';
import { countForView, computeViewCounts } from '../src/features/mail/viewCounts';
import { viewCountId } from '../src/domain/navigation';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { MailMessage, MessageChange } from '../src/domain/mail';

const base = demoWorkspace.messages[0];
const message = (overrides: Partial<MailMessage>): MailMessage => ({ ...base, id: overrides.id ?? 'm', labelIds: [], starred: false, unread: false, ...overrides });
const changes = (field: MessageChange['field'], state: MessageChange['state']): MessageChange[] => [{ field, state, error: null }];

test('view count ids mirror navigation targets and Rust folder_counts view keys', () => {
  assert.equal(viewCountId({ accountId: 'a', mailbox: 'inbox' }), 'kind:inbox');
  assert.equal(viewCountId({ accountId: 'a', mailbox: 'starred' }), 'starred');
  assert.equal(viewCountId({ accountId: 'a', mailbox: 'drafts', draftSource: 'local' }), 'drafts-local');
  assert.equal(viewCountId({ accountId: 'a', mailbox: 'drafts', draftSource: 'server' }), 'kind:drafts');
  assert.equal(viewCountId({ accountId: 'a', mailbox: 'other', folderId: 'f1' }), 'folder:f1');
});

test('countForView sums across accounts for cross-account shortcuts and returns zero entries when loaded', () => {
  const counts = [
    { accountId: 'a', view: 'kind:inbox', total: 3, unread: 2 },
    { accountId: 'b', view: 'kind:inbox', total: 1, unread: 1 },
    { accountId: 'a', view: 'starred', total: 4, unread: 0 },
  ];
  // 计数未加载 → null（侧栏不显示数字，不误报 0）。
  assert.equal(countForView(null, { accountId: 'a', mailbox: 'inbox' }), null);
  // 指定账号精确匹配；缺失视图为零计数。
  assert.deepEqual(countForView(counts, { accountId: 'a', mailbox: 'inbox' }), { accountId: 'a', view: 'kind:inbox', total: 3, unread: 2 });
  assert.deepEqual(countForView(counts, { accountId: 'a', mailbox: 'trash' }), { accountId: 'a', view: 'kind:trash', total: 0, unread: 0 });
  // 跨账号快捷入口对所有账号求和。
  assert.deepEqual(countForView(counts, { accountId: null, mailbox: 'inbox' }), { accountId: '', view: 'kind:inbox', total: 4, unread: 3 });
  assert.deepEqual(countForView(counts, { accountId: null, mailbox: 'starred' }), { accountId: '', view: 'starred', total: 4, unread: 0 });
});

test('computeViewCounts mirrors the mailbox projection: intents move badges like the list does', () => {
  const messages: MailMessage[] = [
    message({ id: 'in1', accountId: 'a', mailbox: 'inbox', unread: true }),
    message({ id: 'in2', accountId: 'a', mailbox: 'inbox', unread: false, starred: true }),
    message({ id: 'del', accountId: 'a', mailbox: 'trash', unread: true, changes: changes('delete', 'queued') }),
    message({ id: 'arc', accountId: 'a', mailbox: 'archive', unread: false, changes: changes('archive', 'queued') }),
    message({ id: 'mv', accountId: 'b', mailbox: 'other', unread: true, changes: changes('move', 'queued') }),
    message({ id: 'd1', accountId: 'a', mailbox: 'drafts', unread: false, draft: { id: 'd1', accountId: 'a', to: '', subject: '', bodyText: '', updatedAt: base.receivedAt } }),
    message({ id: 'sd', accountId: 'a', mailbox: 'drafts', unread: false }),
  ];
  const counts = computeViewCounts(messages);
  const view = (accountId: string, id: string) => counts.find(entry => entry.accountId === accountId && entry.view === id);
  // 账号 a：kind:inbox 2 封 1 未读；kind:drafts 仅服务器草稿（本机草稿不在消息表，单独计 drafts-local）；
  // 星标排除废纸篓投影（in2 唯一星标）。
  assert.deepEqual(view('a', 'kind:inbox'), { accountId: 'a', view: 'kind:inbox', total: 2, unread: 1 });
  assert.deepEqual(view('a', 'kind:drafts'), { accountId: 'a', view: 'kind:drafts', total: 1, unread: 0 });
  assert.deepEqual(view('a', 'drafts-local'), { accountId: 'a', view: 'drafts-local', total: 1, unread: 0 });
  assert.deepEqual(view('a', 'starred'), { accountId: 'a', view: 'starred', total: 1, unread: 0 });
  assert.equal(view('a', 'kind:sent'), undefined);
  // 账号 b：移动意图投影到 kind:other。
  assert.deepEqual(view('b', 'kind:other'), { accountId: 'b', view: 'kind:other', total: 1, unread: 1 });
  assert.equal(view('b', 'kind:inbox'), undefined);
});

test('computeViewCounts counts unread accurately for the sidebar badges', () => {
  const messages: MailMessage[] = Array.from({ length: 7 }, (_, index) => message({ id: `m${index}`, accountId: 'a', mailbox: 'inbox', unread: index % 2 === 0 }));
  const counts = computeViewCounts(messages);
  assert.deepEqual(counts.find(entry => entry.view === 'kind:inbox'), { accountId: 'a', view: 'kind:inbox', total: 7, unread: 4 });
});
