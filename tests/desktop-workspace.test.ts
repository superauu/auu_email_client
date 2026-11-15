import test from 'node:test';
import assert from 'node:assert/strict';
import { desktopMailGateway } from '../src/services/desktop/DesktopMailGateway';
import { desktopMail } from '../src/platform/desktop';
import { demoWorkspace } from '../src/services/demo/fixtures';

test('desktop reads every page within real account scope and excludes legacy demo drafts', async t => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'isTauri');
  Object.defineProperty(globalThis, 'isTauri', { value: true, configurable: true });
  t.after(() => { if (descriptor) Object.defineProperty(globalThis, 'isTauri', descriptor); else Reflect.deleteProperty(globalThis, 'isTauri'); });
  const account = { ...demoWorkspace.accounts[0], id: 'real', status: 'offline' as const };
  const message = { ...demoWorkspace.messages[0], id: 'cached', accountId: account.id };
  const draft = { ...message, id: 'saved-draft', mailbox: 'drafts' as const, draft: { id: 'saved-draft', accountId: account.id, to: '', subject: '', bodyText: '保留正文', updatedAt: message.receivedAt } };
  t.mock.method(desktopMail, 'seed', async () => { assert.fail('desktop must not seed demo data'); });
  t.mock.method(desktopMail, 'importLegacyDrafts', async () => { assert.fail('desktop must not import demo drafts'); });
  t.mock.method(desktopMail, 'workspace', async () => ({ ...demoWorkspace, accounts: [...demoWorkspace.accounts, account], folders: [], syncStates: [] }));
  const calls: Array<{ accountId?: string | null; cursor?: string | null }> = [];
  t.mock.method(desktopMail, 'page', async (query: Parameters<typeof desktopMail.page>[0]) => {
    calls.push(query);
    return query.cursor ? { items: [message], nextCursor: null } : { items: demoWorkspace.messages, nextCursor: 'page-2' };
  });
  t.mock.method(desktopMail, 'drafts', async () => [draft, { ...draft, id: 'legacy-demo-draft', accountId: 'personal' }]);
  const result = await desktopMailGateway.loadWorkspace();
  assert.deepEqual(calls.map(({ accountId, cursor }) => [accountId, cursor]), [['real', null], ['real', 'page-2']]);
  assert.deepEqual(result.accounts.map(item => item.id), ['real']);
  assert.deepEqual(result.messages.map(item => item.id), ['cached', 'saved-draft']);
  assert.equal(result.messages[1].draft?.bodyText, '保留正文');
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
