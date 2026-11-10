import type { MailDraft, MailGateway, MailMessage } from '../../domain/mail';
import { demoWorkspace } from './fixtures';
import { readPreference, writePreference } from '../../lib/preferences';

const KEY = 'auumail.demo.drafts.v1';
export function validateDemoDrafts(raw: unknown): MailDraft[] {
  if (!Array.isArray(raw) || raw.length > 100) throw new Error('Invalid demo drafts');
  return raw.map((item: unknown) => {
    if (!item || typeof item !== 'object') throw new Error('Invalid draft');
    const value = item as Record<string, unknown>;
    for (const key of ['id', 'accountId', 'to', 'subject', 'bodyText', 'updatedAt']) {
      if (typeof value[key] !== 'string' || value[key].length > 50_000) throw new Error('Invalid draft field');
    }
    if (!demoWorkspace.accounts.some(a => a.id === value.accountId) || !String(value.id).startsWith('draft-') || !Number.isFinite(Date.parse(String(value.updatedAt)))) throw new Error('Invalid draft identity');
    if (value.replyToId !== undefined && typeof value.replyToId !== 'string') throw new Error('Invalid reply');
    return {
      id: value.id as string, accountId: value.accountId as string, to: value.to as string,
      subject: value.subject as string, bodyText: value.bodyText as string, updatedAt: value.updatedAt as string,
      ...(value.replyToId ? { replyToId: value.replyToId as string } : {}),
    };
  });
}

function asMessage(draft: MailDraft): MailMessage {
  const account = demoWorkspace.accounts.find(a => a.id === draft.accountId);
  if (!account) throw new Error('找不到发件账号');
  return {
    id: draft.id, accountId: draft.accountId, threadId: draft.replyToId ?? draft.id, mailbox: 'drafts',
    from: { name: account.name, email: account.email }, to: draft.to ? [draft.to] : [], subject: draft.subject || '未命名草稿',
    preview: draft.bodyText.slice(0, 120), bodyText: draft.bodyText, receivedAt: draft.updatedAt,
    unread: false, starred: false, labelIds: [],
  };
}

export const demoMailGateway: MailGateway = {
  mode: 'demo',
  async loadWorkspace(signal) {
    signal?.throwIfAborted();
    const snapshot = structuredClone(demoWorkspace);
    snapshot.messages.push(...(readPreference(KEY, validateDemoDrafts) ?? []).map(asMessage));
    return snapshot;
  },
  async saveDraft(draft) {
    const existing = readPreference(KEY, validateDemoDrafts) ?? [];
    const next = validateDemoDrafts([draft, ...existing.filter(item => item.id !== draft.id)]);
    if (!writePreference(KEY, next)) throw new Error('草稿未能保存，请保留窗口中的内容后重试。');
    return asMessage(draft);
  },
};
