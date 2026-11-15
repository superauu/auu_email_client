import test from 'node:test';
import assert from 'node:assert/strict';
import { isRecipientAddress, recipientError, splitRecipients } from '../src/domain/recipients';
import { validateDraft } from '../src/domain/drafts';

test('recipient paste normalizes display names and common list separators', () => {
  assert.deepEqual(splitRecipients('"Lin, Mei" <mei@example.test>; 团队 <team@example.test>，other@example.test\nlast@example.test'), ['mei@example.test', 'team@example.test', 'other@example.test', 'last@example.test']);
});

test('recipient validation permits cc-only and bcc-only but blocks malformed addresses', () => {
  assert.equal(recipientError(['', '', 'private@example.test']), null);
  assert.equal(recipientError(['', 'team@example.test', '']), null);
  for (const value of ['', 'not-an-address', 'a@localhost', 'a@b@example.test', 'a <b@example.test', 'a@b.test\r\nBcc:x@b.test', '<a@b.test>']) assert.equal(isRecipientAddress(value), false);
  assert.ok(recipientError(['a@example.test', 'broken', '']));
  assert.ok(recipientError(['', '', '']));
});

test('draft validation keeps cc and bcc while accepting legacy drafts and rejecting oversized fields', () => {
  const legacy = { id: 'draft-legacy', accountId: 'work', to: '', subject: '', bodyText: '原文', updatedAt: '2026-11-15T00:00:00Z' };
  assert.equal(validateDraft(legacy).cc, undefined);
  const draft = validateDraft({ ...legacy, cc: 'copy@example.test', bcc: 'private@example.test' });
  assert.equal(draft.cc, 'copy@example.test'); assert.equal(draft.bcc, 'private@example.test');
  assert.throws(() => validateDraft({ ...legacy, cc: 'x'.repeat(1001) }));
  assert.throws(() => validateDraft({ ...legacy, bcc: ['private@example.test'] }));
});
