import test from 'node:test';
import assert from 'node:assert/strict';
import { realWorkspace } from '../src/domain/workspace';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { WorkspaceSnapshot } from '../src/domain/mail';
import { selectMessages } from '../src/features/mail/selectors';

test('real workspace excludes legacy demo data across all collections without modifying storage input', () => {
  const real = { ...demoWorkspace.accounts[0], id: 'added', status: 'offline' as const };
  const message = { ...demoWorkspace.messages[0], id: 'real-message', accountId: real.id, labelIds: ['my-label'] };
  const snapshot: WorkspaceSnapshot = {
    ...structuredClone(demoWorkspace),
    accounts: [...demoWorkspace.accounts, real, { ...real, id: 'reauth', status: 'reauthorization-required' }],
    messages: [...demoWorkspace.messages, message, { ...message, id: 'orphan', accountId: 'missing' }],
    labels: [...demoWorkspace.labels, { id: 'my-label', name: '我的标签', color: 'teal', scope: 'local' },
      { id: 'owned-label', name: '账号标签', color: 'blue', scope: 'server', accountId: real.id },
      { id: 'demo-label', name: '演示标签', color: 'blue', scope: 'server', accountId: 'personal' }],
    folders: ['personal', real.id, 'missing'].map(accountId => ({ id: accountId, accountId, name: 'INBOX', kind: 'inbox', available: true })),
    events: [...demoWorkspace.events, { ...demoWorkspace.events[0], id: 'real-event', accountId: real.id, messageId: message.id }],
    syncStates: { personal: 'demo-time', added: 'real-time', missing: 'orphan-time' },
  };
  const before = structuredClone(snapshot);
  const result = realWorkspace(snapshot);
  assert.deepEqual(result.accounts.map(account => account.id), ['added', 'reauth']);
  assert.deepEqual(result.messages.map(item => item.id), ['real-message']);
  assert.deepEqual(result.folders?.map(folder => folder.accountId), ['added']);
  assert.deepEqual(result.labels.map(label => label.id), ['my-label', 'owned-label']);
  assert.deepEqual(result.events.map(event => event.id), ['real-event']);
  assert.deepEqual(result.syncStates, { added: 'real-time' });
  assert.equal(selectMessages(result.messages, { accountId: null, mailbox: 'inbox', labelId: null, search: '' }).length, 1);
  assert.deepEqual(snapshot, before);
});

test('no added accounts yields an empty desktop workspace, never a demo fallback', () => {
  assert.deepEqual(realWorkspace(demoWorkspace), { accounts: [], messages: [], folders: [], labels: [], events: [], syncStates: {} });
});
