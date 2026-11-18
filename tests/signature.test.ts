import test from 'node:test';
import assert from 'node:assert/strict';
import { validateAccountSignature, MAX_BODY_HTML, MAX_SIGNATURE_IMAGE } from '../src/domain/signature';
import { validateDraft } from '../src/domain/drafts';

const base = { id: 'draft-a', accountId: 'work', to: '', subject: '', bodyText: '', updatedAt: new Date().toISOString() };

test('signature defaults roundtrip and reject invalid shapes or oversize html', () => {
  const value = validateAccountSignature({ html: '<p>张三</p>', useNew: true, useReply: false });
  assert.deepEqual(value, { html: '<p>张三</p>', useNew: true, useReply: false });
  assert.throws(() => validateAccountSignature(null));
  assert.throws(() => validateAccountSignature({ html: 3, useNew: true, useReply: true }));
  assert.throws(() => validateAccountSignature({ html: '<p>x</p>', useNew: 'yes', useReply: true }));
  assert.throws(() => validateAccountSignature({ html: '<p>'.repeat(MAX_BODY_HTML), useNew: true, useReply: true }));
  // 上限边界之内可行（镜像 Rust MAX_BODY_HTML）。
  assert.doesNotThrow(() => validateAccountSignature({ html: 'x'.repeat(MAX_BODY_HTML), useNew: true, useReply: true }));
});

test('body_html draft limit matches the signature image budget', () => {
  // 2M 上限（签名图片 base64 内嵌所需）；旧 200k 会拒绝含图签名草稿。
  assert.equal(MAX_BODY_HTML, 2_000_000);
  assert.equal(MAX_SIGNATURE_IMAGE, 1024 * 1024);
  const big = validateDraft({ ...base, bodyHtml: 'x'.repeat(MAX_BODY_HTML) });
  assert.equal(big.bodyHtml?.length, MAX_BODY_HTML);
  assert.throws(() => validateDraft({ ...base, bodyHtml: 'x'.repeat(MAX_BODY_HTML + 1) }));
});
