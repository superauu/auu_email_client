import type { MailAccount, MailDraft, MailMessage, DraftSaveOptions } from '../../domain/mail';
import { demoWorkspace } from './fixtures';
import { readPreference, writePreference } from '../../lib/preferences';
import { validateDraft, draftAsMessage } from '../../domain/drafts';
export { draftAsMessage } from '../../domain/drafts';

const KEY = 'auumail.demo.drafts.v1';
export function validateDemoDrafts(raw: unknown): MailDraft[] {
  if (!Array.isArray(raw) || raw.length > 100) throw new Error('Invalid demo drafts');
  return raw.map((item: unknown) => {
    const draft = validateDraft(item);
    if (!demoWorkspace.accounts.some(a => a.id === draft.accountId)) throw new Error('Invalid demo account');
    return draft;
  });
}

/** Legacy desktop draft source; retained as a backup after transactional native import. */
export function loadLegacyDemoDrafts(): MailDraft[] {
  return readPreference(KEY, validateDemoDrafts) ?? [];
}

/** Browser demo only. Desktop drafts are persisted by the native repository. */
export function loadDraftMessages(accounts: MailAccount[]): MailMessage[] {
  return (readPreference(KEY, validateDemoDrafts) ?? []).map(draft => draftAsMessage(draft, accounts));
}

export function persistDraft(draft: MailDraft, options?: DraftSaveOptions): MailDraft {
  const existing = readPreference(KEY, validateDemoDrafts) ?? [];
  const current = existing.find(item => item.id === draft.id);
  const expectedRevision = draft.revision ?? 0;
  if (current ? (current.revision ?? 0) !== expectedRevision || current.accountId !== (options?.previousAccountId ?? draft.accountId) : expectedRevision !== 0) {
    throw { code: 'conflict', message: '草稿已在其他窗口修改，请保留当前内容并重新打开草稿。', retryable: false };
  }
  const saved = validateDraft({ ...draft, revision: expectedRevision + 1, updatedAt: new Date().toISOString() });
  const next = validateDemoDrafts([saved, ...existing.filter(item => item.id !== draft.id)]);
  if (!writePreference(KEY, next)) throw new Error('草稿未能保存，请保留窗口中的内容后重试。');
  return saved;
}
