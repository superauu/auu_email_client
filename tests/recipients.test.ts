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

test('draft attachment validation round-trips metadata and enforces count and size limits', () => {
  const legacy = { id: 'draft-legacy', accountId: 'work', to: '', subject: '', bodyText: '', updatedAt: '2026-11-16T00:00:00Z' };
  const attachment = { id: 'att-1', name: '季度报告.pdf', size: 2048, mediaType: 'application/pdf', sha256: 'a'.repeat(64) };
  // 旧草稿无附件字段 → 投影不带 attachments。
  assert.equal(validateDraft(legacy).attachments, undefined);
  // 空数组等价于无附件。
  assert.equal(validateDraft({ ...legacy, attachments: [] }).attachments, undefined);
  // 合法元数据逐字段保留（顺序与内容一致）。
  const saved = validateDraft({ ...legacy, attachments: [attachment] });
  assert.deepEqual(saved.attachments, [attachment]);
  // 名称 / id / sha256 / mediaType / size 字段级校验。
  for (const broken of [
    { ...attachment, id: 'bad' },
    { ...attachment, name: '  ' },
    { ...attachment, name: 'x'.repeat(241) },
    { ...attachment, sha256: 'zz' },
    { ...attachment, mediaType: '' },
    { ...attachment, size: -1 },
    { ...attachment, size: 17 * 1024 * 1024 },
  ]) assert.throws(() => validateDraft({ ...legacy, attachments: [broken] }), JSON.stringify(broken));
  // 数量上限：21 个拒绝，20 个（合计不超 16 MiB 内的单件）通过投影。
  const many = Array.from({ length: 21 }, (_, index) => ({ ...attachment, id: `att-${index}`, size: 1024 }));
  assert.throws(() => validateDraft({ ...legacy, attachments: many }));
  assert.equal(validateDraft({ ...legacy, attachments: many.slice(0, 20) }).attachments?.length, 20);
  // 合计上限：20 × 1 MiB > 16 MiB 拒绝；16 × 1 MiB 恰好通过。
  const heavy = Array.from({ length: 20 }, (_, index) => ({ ...attachment, id: `att-${index}`, size: 1024 * 1024 }));
  assert.throws(() => validateDraft({ ...legacy, attachments: heavy }));
  assert.equal(validateDraft({ ...legacy, attachments: heavy.slice(0, 16) }).attachments?.length, 16);
});
