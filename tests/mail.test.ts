import test from 'node:test';
import assert from 'node:assert/strict';
import { selectMessages } from '../src/features/mail/selectors';
import { replyTargets } from '../src/domain/mail';
import { demoWorkspace } from '../src/services/demo/fixtures';
import { demoMailGateway } from '../src/services/demo/DemoMailGateway';
import { validateDemoDrafts } from '../src/services/demo/drafts';
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
    const first = await demoMailGateway.saveDraft(draft);
    const second = await demoMailGateway.saveDraft({ ...first.draft!, bodyText: 'Updated' });
    await assert.rejects(demoMailGateway.saveDraft({ ...draft, bodyText: 'Stale overwrite' }), { code: 'conflict' });
    const workspace = await demoMailGateway.loadWorkspace();
    const drafts = workspace.messages.filter(message => message.id === draft.id);
    assert.equal(drafts.length, 1);
    assert.equal(drafts[0].bodyText, 'Updated');
    storage.setItem = () => { throw new Error('Quota'); };
    await assert.rejects(demoMailGateway.saveDraft({ ...second.draft!, bodyText: 'Unsaved' }), /未能保存/);
  } finally { Reflect.deleteProperty(globalThis, 'localStorage'); }
});

test('aborted workspace requests reject', async () => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(demoMailGateway.loadWorkspace(controller.signal));
});


test('unread filtering combines with account folder label and search without changing mail',()=>{
  const messages=demoWorkspace.messages.map((message,index)=>({...message,folderIds:['folder'],unread:index===0}));
  const original=structuredClone(messages);
  const selected=messages[0];
  assert.deepEqual(selectMessages(messages,{...query,unreadOnly:true,accountId:selected.accountId,folderId:'folder',search:selected.subject}),[selected]);
  assert.equal(selectMessages(messages,{...query,unreadOnly:true,accountId:'other',folderId:'folder',search:selected.subject}).length,0);
  assert.deepEqual(messages,original);
});

test('replyTargets: normal reply prefers Reply-To and falls back to the sender', () => {
  const base = { from: { name: 'A', email: 'a@example.test' }, to: ['me@example.test'] };
  assert.deepEqual(replyTargets({ ...base, replyTo: ['list@lists.example.test'] }, 'me@example.test', 'reply'), {
    to: ['list@lists.example.test'], cc: [],
  });
  // Reply-To 只含本账号地址时退回发件人。
  assert.deepEqual(replyTargets({ ...base, replyTo: ['me@example.test'] }, 'me@example.test', 'reply'), {
    to: ['a@example.test'], cc: [],
  });
  assert.deepEqual(replyTargets(base, 'me@example.test', 'reply'), { to: ['a@example.test'], cc: [] });
});

test('replyTargets: reply-all merges To and Cc, drops own address and duplicates', () => {
  const message = {
    from: { name: 'A', email: 'a@example.test' },
    to: ['me@example.test', 'A@EXAMPLE.TEST', 'b@example.test'],
    cc: ['me@example.test', 'c@example.test', 'b@example.test'],
  };
  assert.deepEqual(replyTargets(message, 'me@example.test', 'reply-all'), {
    to: ['a@example.test', 'b@example.test'], cc: ['c@example.test'],
  });
});

test('replyTargets: reply-all keeps at least one recipient even for self-sent mail', () => {
  const message = {
    from: { name: 'Me', email: 'me@example.test' },
    to: ['b@example.test'],
    cc: ['c@example.test'],
  };
  // 发件人是自己：目标降级为其余收件人，Cc 保持独立。
  assert.deepEqual(replyTargets(message, 'me@example.test', 'reply-all'), {
    to: ['b@example.test'], cc: ['c@example.test'],
  });
});
