import type { MailAccount, MailDraft, MailMessage } from './mail';

export function validateDraft(raw: unknown): MailDraft {
  if (!raw || typeof raw !== 'object') throw new Error('草稿格式无效');
  const value = raw as Record<string, unknown>;
  const fields = { id: 100, accountId: 200, to: 1000, subject: 1000, bodyText: 50000, updatedAt: 64 };
  for (const [key, limit] of Object.entries(fields)) {
    if (typeof value[key] !== 'string' || value[key].length > limit) throw new Error('草稿字段无效或超出长度限制');
  }
  if (!/^draft-[a-zA-Z0-9_-]+$/.test(value.id as string) || !value.accountId || !Number.isFinite(Date.parse(value.updatedAt as string))) throw new Error('草稿标识或时间无效');
  if (value.replyToId !== undefined && (typeof value.replyToId !== 'string' || value.replyToId.length === 0 || value.replyToId.length > 200)) throw new Error('回复关联无效');
  const revision = value.revision ?? 0;
  if (typeof revision !== 'number' || !Number.isSafeInteger(revision) || revision < 0 || revision > 1_000_000_000) throw new Error('草稿版本无效');
  return {
    id: value.id as string, accountId: value.accountId as string, to: value.to as string,
    subject: value.subject as string, bodyText: value.bodyText as string, updatedAt: value.updatedAt as string, revision,
    ...(value.replyToId ? { replyToId: value.replyToId as string } : {}),
  };
}

export function draftAsMessage(draft: MailDraft, accounts: MailAccount[]): MailMessage {
  const account = accounts.find(item => item.id === draft.accountId);
  if (!account) throw new Error('发件账号已不存在，请选择其他账号。');
  return {
    id: draft.id, accountId: draft.accountId, threadId: draft.replyToId ?? draft.id, mailbox: 'drafts',
    from: { name: account.name, email: account.email }, to: draft.to ? [draft.to] : [], subject: draft.subject || '未命名草稿',
    preview: draft.bodyText.slice(0, 120), bodyText: draft.bodyText, receivedAt: draft.updatedAt,
    unread: false, starred: false, labelIds: [], draft: { ...draft },
  };
}
