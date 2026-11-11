import test from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import type { MailDraft, DraftSaveOptions } from '../src/domain/mail';
import { DraftSession } from '../src/features/compose/DraftSession';
import { draftAsMessage } from '../src/domain/drafts';
import { demoWorkspace } from '../src/services/demo/fixtures';

const initial: MailDraft = { id: 'draft-session', accountId: 'personal', to: '', subject: '', bodyText: '', updatedAt: '2026-11-11T00:00:00Z' };
const saved = (draft: MailDraft): MailDraft => ({ ...draft, revision: (draft.revision ?? 0) + 1 });

test('edits arriving during a save are serialized and remain in the latest draft', async () => {
  const writes: { draft: MailDraft; options: DraftSaveOptions }[] = [];
  let resolveFirst!: (value: MailDraft) => void;
  const first = new Promise<MailDraft>(resolve => { resolveFirst = resolve; });
  const session = new DraftSession(initial, async (draft, options) => {
    writes.push({ draft: { ...draft }, options });
    return writes.length === 1 ? first : saved(draft);
  });
  session.update({ bodyText: 'First version' });
  const pending = session.flush();
  session.update({ bodyText: 'Latest version', accountId: 'work' });
  assert.equal(writes.length, 1);
  resolveFirst(saved(writes[0].draft));
  assert.equal(await pending, true);
  assert.equal(writes.length, 2);
  assert.equal(writes[1].draft.revision, 1);
  assert.equal(writes[1].options.previousAccountId, 'personal');
  assert.equal(writes[1].draft.accountId, 'work');
  assert.equal(session.getSnapshot().draft.bodyText, 'Latest version');
  assert.equal(session.getSnapshot().draft.revision, 2);
  assert.equal(session.needsProtection(), false);
  session.cancelScheduledSave();
});

test('failed saves preserve input and can be retried without silently incrementing revision', async () => {
  let fail = true;
  const session = new DraftSession(initial, async draft => {
    if (fail) throw new Error('Disk unavailable');
    assert.equal(draft.revision, 0);
    return saved(draft);
  });
  session.update({ subject: 'Keep this', bodyText: 'Do not lose this text' });
  assert.equal(await session.flush(), false);
  assert.equal(session.getSnapshot().error, 'Disk unavailable');
  assert.equal(session.getSnapshot().draft.bodyText, 'Do not lose this text');
  assert.equal(session.needsProtection(), true);
  fail = false;
  assert.equal(await session.flush(), true);
  assert.equal(session.getSnapshot().error, null);
});

test('autosave is debounced, and pausing prevents a pending save after discard', async () => {
  const writes: MailDraft[] = [];
  const session = new DraftSession(initial, async draft => { writes.push(draft); return saved(draft); }, 5);
  session.update({ subject: 'One' });
  session.update({ subject: 'Two' });
  await delay(30);
  assert.equal(writes.length, 1);
  assert.equal(writes[0].subject, 'Two');
  session.update({ subject: 'Discard this edit' });
  session.pause();
  await delay(30);
  assert.equal(writes.length, 1);
  session.resume();
  await delay(30);
  assert.equal(writes.length, 2);
  session.cancelScheduledSave();
});

test('an invalid save response cannot acknowledge the edited buffer', async () => {
  const session = new DraftSession(initial, async draft => ({ ...draft, id: 'draft-other', revision: 1 }));
  session.update({ bodyText: 'Still here' });
  assert.equal(await session.flush(), false);
  assert.equal(session.getSnapshot().draft.bodyText, 'Still here');
  assert.equal(session.getSnapshot().dirty, true);
});

test('draft projection preserves an empty subject and reply identity for reopening', () => {
  const draft = { ...initial, replyToId: 'design-review', accountId: 'work', revision: 2 };
  const message = draftAsMessage(draft, demoWorkspace.accounts);
  assert.equal(message.subject, '未命名草稿');
  assert.equal(message.draft?.subject, '');
  assert.equal(message.draft?.replyToId, 'design-review');
  assert.equal(message.draft?.revision, 2);
});
