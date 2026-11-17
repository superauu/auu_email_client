import test from 'node:test';
import assert from 'node:assert/strict';
import { accountDirectories, isNavigationActive, navigationKey, validateNavigationPreferences } from '../src/domain/navigation';
import { selectMessages } from '../src/features/mail/selectors';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { MailFolder, MailMessage, MailQuery } from '../src/domain/mail';

const account = { ...demoWorkspace.accounts[0], id: 'real', status: 'offline' as const };
const folder = (id: string, name: string, kind: string, accountId = 'real'): MailFolder => ({ id, name, kind, accountId, available: true });

test('account directories map standard names once and retain every physical folder ID', () => {
  const remote = [folder('in', 'INBOX', 'inbox'), folder('drafts', 'Drafts', 'drafts'), folder('trash', 'Deleted Messages', 'trash'), folder('project', 'Projects/2026', 'other'), folder('wrong', 'INBOX', 'inbox', 'other-account')];
  const entries = accountDirectories(account, remote, []);
  // 废纸篓与归档同为本机意图视图：唯一远端目录不单独展示物理副本。
  assert.deepEqual(entries.filter(entry => entry.target.folderId).map(entry => [entry.label, entry.target.folderId]), [['收件箱', 'in'], ['服务器草稿', 'drafts'], ['Projects/2026', 'project']]);
  const trashView = entries.filter(entry => entry.kind === 'trash');
  assert.deepEqual(trashView.map(entry => [entry.label, entry.target.folderId ?? null]), [['废纸篓', null]]);
  assert.equal(entries.filter(entry => entry.kind === 'inbox').length, 1);
  assert.equal(entries.filter(entry => entry.label === '本机草稿').length, 1);
  assert.ok(entries.every(entry => entry.target.accountId === 'real'));
  const two = accountDirectories(account, [...remote, folder('other-trash', 'Trash', 'trash')], []);
  // 与归档一致：意图视图 + 每个物理目录各一条（标签带目录名区分）。
  assert.deepEqual(two.filter(entry => entry.kind === 'trash').map(entry => entry.label), ['废纸篓', '废纸篓 · Deleted Messages', '废纸篓 · Trash']);
});

test('local and server drafts remain separate and folder queries do not rely on mailbox projection', () => {
  const original = demoWorkspace.messages[0];
  const local: MailMessage = { ...original, id: 'draft-local', accountId: 'real', mailbox: 'drafts', folderIds: [], draft: { id: 'draft-local', accountId: 'real', to: '', subject: '', bodyText: '', updatedAt: original.receivedAt } };
  const server: MailMessage = { ...original, id: 'server', accountId: 'real', mailbox: 'drafts', folderIds: ['drafts'] };
  const secondAccount = { ...server, id: 'other', accountId: 'other-account' };
  const messages = [local, server, secondAccount];
  const query: MailQuery = { accountId: 'real', mailbox: 'drafts', labelId: null, search: '' };
  assert.deepEqual(selectMessages(messages, { ...query, draftSource: 'local' }), [local]);
  assert.deepEqual(selectMessages(messages, { ...query, draftSource: 'server' }), [server]);
  assert.deepEqual(selectMessages([{ ...server, mailbox: 'inbox' }], { ...query, folderId: 'drafts', draftSource: 'server' }).map(item => item.id), ['server']);
});

test('navigation keys preserve opaque identifiers and only the matching account location is active', () => {
  const first = { accountId: 'account:一', mailbox: 'other' as const, folderId: 'folder:/[A],"B"' };
  const second = { ...first, accountId: 'account:二' };
  assert.notEqual(navigationKey(first), navigationKey(second));
  assert.ok(isNavigationActive({ ...first, labelId: null, search: '', unreadOnly: true }, first));
  assert.equal(isNavigationActive({ ...second, labelId: null, search: '' }, first), false);
  assert.equal(isNavigationActive({ ...first, labelId: 'tag', search: '' }, first), false);
});

test('account inbox respects pending local archive and archive remains available before MOVE completes', () => {
  const pending: MailMessage = { ...demoWorkspace.messages[0], accountId: 'real', mailbox: 'archive', folderIds: ['in', 'project'], changes: [{ field: 'archive', state: 'unknown', error: 'pending confirmation' }] };
  const entries = accountDirectories(account, [folder('in', 'INBOX', 'inbox'), folder('archive', 'Archive', 'archive'), folder('project', 'Project', 'other')], [pending]);
  const select = (name: string) => selectMessages([pending], { ...entries.find(entry => entry.label === name)!.target, labelId: null, search: '' });
  assert.deepEqual(select('收件箱'), []);
  assert.deepEqual(select('归档'), [pending]);
  assert.deepEqual(select('Project'), [pending]);
});

test('navigation preferences reject malformed or excessive pins and keep collapsed state', () => {
  const target = { accountId: 'real', mailbox: 'drafts', draftSource: 'local' };
  assert.deepEqual(validateNavigationPreferences({ pinned: [target, target], expanded: [] }), { pinned: [{ ...target, folderId: undefined }], expanded: [] });
  for (const raw of [null, { pinned: [], expanded: 'all' }, { pinned: Array(31).fill(target), expanded: null }, { pinned: [{ ...target, mailbox: 'constructor' }], expanded: null }, { pinned: [{ ...target, folderId: 'server-id' }], expanded: null }, { pinned: [{ ...target, accountId: null }], expanded: null }]) assert.throws(() => validateNavigationPreferences(raw));
});


test('delete and move intents project to trash and target folder kinds', () => {
  const base: MailMessage = { ...demoWorkspace.messages[0], id: 'm1', accountId: 'real', mailbox: 'inbox', folderIds: ['in'], changes: [] };
  const deleted: MailMessage = { ...base, mailbox: 'trash', changes: [{ field: 'delete', state: 'queued', error: null }] };
  const moved: MailMessage = { ...base, mailbox: 'other', changes: [{ field: 'move', state: 'queued', error: null }] };
  assert.deepEqual(selectMessages([deleted], { accountId: 'real', mailbox: 'trash', labelId: null, search: '' }).map(item => item.id), ['m1']);
  assert.deepEqual(selectMessages([deleted], { accountId: 'real', mailbox: 'inbox', labelId: null, search: '' }), []);
  assert.deepEqual(selectMessages([moved], { accountId: 'real', mailbox: 'other', labelId: null, search: '' }).map(item => item.id), ['m1']);
  // 删除意图的邮件不再出现在星标（与归档意图一致的排除规则由 Rust 投影保证；此处验证 mailbox 投影可筛选）。
  assert.deepEqual(selectMessages([moved], { accountId: 'real', mailbox: 'trash', labelId: null, search: '' }), []);
});
