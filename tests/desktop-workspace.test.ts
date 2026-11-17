import test from 'node:test';
import assert from 'node:assert/strict';
import { desktopMailGateway } from '../src/services/desktop/DesktopMailGateway';
import { desktopMail } from '../src/platform/desktop';
import { demoWorkspace } from '../src/services/demo/fixtures';

test('desktop workspace loads base plus drafts only and messages arrive per view (no startup drain)', async t => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'isTauri');
  Object.defineProperty(globalThis, 'isTauri', { value: true, configurable: true });
  t.after(() => { if (descriptor) Object.defineProperty(globalThis, 'isTauri', descriptor); else Reflect.deleteProperty(globalThis, 'isTauri'); });
  const account = { ...demoWorkspace.accounts[0], id: 'real', status: 'offline' as const };
  const draft = { ...demoWorkspace.messages[0], id: 'saved-draft', accountId: account.id, mailbox: 'drafts' as const, draft: { id: 'saved-draft', accountId: account.id, to: '', subject: '', bodyText: '保留正文', updatedAt: demoWorkspace.messages[0].receivedAt } };
  t.mock.method(desktopMail, 'seed', async () => { assert.fail('desktop must not seed demo data'); });
  t.mock.method(desktopMail, 'importLegacyDrafts', async () => { assert.fail('desktop must not import demo drafts'); });
  t.mock.method(desktopMail, 'workspace', async () => ({ ...demoWorkspace, accounts: [...demoWorkspace.accounts, account], folders: [], syncStates: [] }));
  // 启动不再全量分页灌入邮件：普通邮件一律按视图懒加载，这里调用 page 即为回归。
  t.mock.method(desktopMail, 'page', async () => { assert.fail('workspace load must not drain message pages'); });
  t.mock.method(desktopMail, 'drafts', async () => [draft, { ...draft, id: 'legacy-demo-draft', accountId: 'personal' }]);
  const result = await desktopMailGateway.loadWorkspace();
  assert.deepEqual(result.accounts.map(item => item.id), ['real']);
  // 仅本机草稿进入内存；遗留演示账号草稿被过滤。
  assert.deepEqual(result.messages.map(item => item.id), ['saved-draft']);
  assert.equal(result.messages[0].draft?.bodyText, '保留正文');
});

test('desktop loadView maps the UI query onto the paged store query', async t => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'isTauri');
  Object.defineProperty(globalThis, 'isTauri', { value: true, configurable: true });
  t.after(() => { if (descriptor) Object.defineProperty(globalThis, 'isTauri', descriptor); else Reflect.deleteProperty(globalThis, 'isTauri'); });
  const calls: unknown[] = [];
  t.mock.method(desktopMail, 'page', async (query: Parameters<typeof desktopMail.page>[0]) => {
    calls.push(query);
    return { items: [], nextCursor: null };
  });
  await desktopMailGateway.loadView?.({ accountId: 'real', mailbox: 'inbox', labelId: null, search: '' });
  await desktopMailGateway.loadView?.({ accountId: 'real', mailbox: 'starred', labelId: null, search: '发票', unreadOnly: true }, 'cursor-1');
  await desktopMailGateway.loadView?.({ accountId: 'real', mailbox: 'drafts', draftSource: 'local', labelId: null, search: '' });
  await desktopMailGateway.loadView?.({ accountId: null, mailbox: 'other', folderId: 'folder-1', labelId: null, search: '' });
  assert.deepEqual(calls, [
    { accountId: 'real', mailbox: 'inbox', folderId: null, labelId: null, search: null, cursor: null, limit: 200 },
    // 星标视图与搜索词直达分页查询；unreadOnly 是客户端过滤，不进入服务端查询。
    { accountId: 'real', mailbox: 'starred', folderId: null, labelId: null, search: '发票', cursor: 'cursor-1', limit: 200 },
    // 本机草稿视图不触发服务端分页（草稿随工作区全量加载）。
    { accountId: null, mailbox: 'other', folderId: 'folder-1', labelId: null, search: null, cursor: null, limit: 200 },
  ]);
});

test('desktop without real accounts does not read demo mail or drafts', async t => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'isTauri');
  Object.defineProperty(globalThis, 'isTauri', { value: true, configurable: true });
  t.after(() => { if (descriptor) Object.defineProperty(globalThis, 'isTauri', descriptor); else Reflect.deleteProperty(globalThis, 'isTauri'); });
  t.mock.method(desktopMail, 'workspace', async () => ({ ...demoWorkspace, folders: [], syncStates: [] }));
  t.mock.method(desktopMail, 'page', async () => { assert.fail('no account scope to query'); });
  t.mock.method(desktopMail, 'drafts', async () => { assert.fail('no real drafts to load'); });
  t.mock.method(desktopMail, 'seed', async () => { assert.fail('must not create demo accounts'); });
  t.mock.method(desktopMail, 'importLegacyDrafts', async () => { assert.fail('must not import demo drafts'); });
  const snapshot = await desktopMailGateway.loadWorkspace();
  assert.deepEqual(snapshot, { accounts: [], messages: [], labels: [], events: [], folders: [], syncStates: {} });
});
