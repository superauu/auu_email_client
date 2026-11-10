import test from 'node:test';
import assert from 'node:assert/strict';
import { selectMessages } from '../src/features/mail/selectors';
import { demoWorkspace } from '../src/services/demo/fixtures';
import { demoMailGateway, validateDemoDrafts } from '../src/services/demo/DemoMailGateway';
import type { MailDraft, MailQuery } from '../src/domain/mail';

const query: MailQuery = { accountId: null, mailbox: 'inbox', labelId: null, search: '' };

test('combined account, mailbox and search filters cannot leak another account', () => {
  const result = selectMessages(demoWorkspace.messages, { ...query, accountId: 'work', search: '阅读体验' });
  assert.equal(result.length, 2);
  assert.ok(result.every(message => message.accountId === 'work'));
  assert.equal(selectMessages(demoWorkspace.messages, { ...query, accountId: 'other', search: '阅读体验' }).length, 0);
});

test('local label searches span folders and exclude trash', () => {
  const trash = { ...demoWorkspace.messages[0], id: 'trash', mailbox: 'trash' as const };
  const result = selectMessages([...demoWorkspace.messages, trash], { ...query, labelId: 'design' });
  assert.ok(result.some(message => message.mailbox === 'archive'));
  assert.ok(!result.some(message => message.id === 'trash'));
});

test('unknown accounts, corrupt persisted drafts and invalid dates are rejected', () => {
  assert.throws(() => validateDemoDrafts([{ id: 'draft-a', accountId: 'unknown', to: '', subject: '', bodyText: '', updatedAt: new Date().toISOString() }]));
  assert.throws(() => validateDemoDrafts([{}]));
  assert.throws(() => validateDemoDrafts('bad'));
  assert.throws(() => validateDemoDrafts([{ id: 'draft-a', accountId: 'work', to: '', subject: '', bodyText: '', updatedAt: 'yesterday' }]));
});

test('drafts persist, update in place, and storage failures reject the save', async () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
  };
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
  const draft: MailDraft = { id: 'draft-test', accountId: 'personal', to: 'friend@example.test', subject: 'Draft', bodyText: 'First', updatedAt: '2026-11-10T00:00:00Z' };
  try {
    await demoMailGateway.saveDraft(draft);
    await demoMailGateway.saveDraft({ ...draft, bodyText: 'Updated' });
    const workspace = await demoMailGateway.loadWorkspace();
    const drafts = workspace.messages.filter(message => message.id === draft.id);
    assert.equal(drafts.length, 1);
    assert.equal(drafts[0].bodyText, 'Updated');
    storage.setItem = () => { throw new Error('Quota'); };
    await assert.rejects(demoMailGateway.saveDraft({ ...draft, bodyText: 'Unsaved' }));
  } finally { Reflect.deleteProperty(globalThis, 'localStorage'); }
});

test('aborted workspace requests reject', async () => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(demoMailGateway.loadWorkspace(controller.signal));
});
