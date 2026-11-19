import test from 'node:test';
import assert from 'node:assert/strict';
import {
  captureQuery,
  MAX_SAVED_SEARCHES,
  parseSavedSearch,
  parseSavedSearches,
  sanitizeSavedQuery,
  savedSearchNameError,
  sameQuery,
} from '../src/domain/savedSearch';
import type { MailQuery } from '../src/domain/mail';

const baseQuery: MailQuery = { accountId: null, mailbox: 'inbox', labelId: null, search: '发票' };

test('saved search name validation', () => {
  assert.equal(savedSearchNameError('季度发票'), '');
  assert.notEqual(savedSearchNameError('   '), '');
  assert.notEqual(savedSearchNameError('名'.repeat(65)), '');
  assert.equal(savedSearchNameError('名'.repeat(64)), '');
});

test('captureQuery snapshots structured fields and drops runtime state', () => {
  const captured = captureQuery({ ...baseQuery, accountId: 'acc-1', folderId: 'f1', unreadOnly: true, draftSource: 'local' });
  assert.deepEqual(captured, { accountId: 'acc-1', mailbox: 'inbox', folderId: 'f1', labelId: null, search: '发票', unreadOnly: true, draftSource: 'local' });
  const minimal = captureQuery(baseQuery);
  assert.deepEqual(minimal, { accountId: null, mailbox: 'inbox', folderId: null, labelId: null, search: '发票' });
  assert.ok(!('unreadOnly' in minimal) && !('draftSource' in minimal));
});

test('parseSavedSearch drops malformed entries and normalizes', () => {
  const good = { id: 'search:a', name: ' 发票 ', query: { mailbox: 'inbox', accountId: 'a', folderId: null, labelId: null, search: 'x', unreadOnly: true, draftSource: 'bogus' } };
  const parsed = parseSavedSearch(good);
  assert.ok(parsed);
  assert.equal(parsed.name, '发票');
  assert.equal(parsed.query.draftSource, undefined);
  assert.equal(parseSavedSearch(null), null);
  assert.equal(parseSavedSearch({ id: 'x', name: '', query: { mailbox: 'inbox' } }), null);
  assert.equal(parseSavedSearch({ id: 'x', name: 'n', query: { mailbox: 'nope' } }), null);
  // 上限截断：只保留前 N 个。
  const many = Array.from({ length: MAX_SAVED_SEARCHES + 5 }, (_, index) => ({ id: `s${index}`, name: `n${index}`, query: { mailbox: 'inbox' } }));
  assert.equal(parseSavedSearches(many).length, MAX_SAVED_SEARCHES);
  assert.equal(parseSavedSearches('nope').length, 0);
});

test('sanitizeSavedQuery resets stale account, folder and label references', () => {
  const shape = {
    accountIds: new Set(['acc-1']),
    folders: [{ id: 'f1', accountId: 'acc-1' }, { id: 'f2', accountId: 'acc-2' }],
    labelIds: new Set(['lbl:1']),
  };
  const sanitized = sanitizeSavedQuery({ accountId: 'gone', mailbox: 'inbox', folderId: 'f2', labelId: 'lbl:1', search: 'x' }, shape);
  // 账号失效 → 重置；目录不再属于该（已失效）账号 → 重置；标签仍有效保留。
  assert.deepEqual({ accountId: sanitized.accountId, folderId: sanitized.folderId, labelId: sanitized.labelId }, { accountId: null, folderId: null, labelId: 'lbl:1' });
  const kept = sanitizeSavedQuery({ accountId: 'acc-1', mailbox: 'other', folderId: 'f1', labelId: null, search: '' }, shape);
  assert.deepEqual({ accountId: kept.accountId, folderId: kept.folderId, mailbox: kept.mailbox }, { accountId: 'acc-1', folderId: 'f1', mailbox: 'other' });
});

test('sameQuery compares current query with saved snapshot', () => {
  const saved = { accountId: null, mailbox: 'inbox' as const, folderId: null, labelId: null, search: '发票', unreadOnly: true };
  assert.ok(sameQuery({ ...baseQuery, unreadOnly: true }, saved));
  assert.ok(!sameQuery(baseQuery, saved));
  assert.ok(!sameQuery({ ...baseQuery, unreadOnly: true }, { ...saved, search: 'x' }));
});
