import type { DraftAttachment, MailAccount, MailDraft, MailMessage } from './mail';
import { t } from '../i18n/core';

/** 附件限制与 Rust 侧对称：最多 20 个、原始字节合计 16 MiB（base64 后约 21.3 MiB）。 */
export const MAX_ATTACHMENTS = 20;
export const MAX_ATTACHMENT_BYTES = 16 * 1024 * 1024;

function validateAttachments(raw: unknown): DraftAttachment[] {
  if (raw === undefined) return [];
  if (!Array.isArray(raw) || raw.length > MAX_ATTACHMENTS) throw new Error(t('draftErr.attachmentCount'));
  const attachments: DraftAttachment[] = [];
  let total = 0;
  for (const item of raw) {
    if (!item || typeof item !== 'object') throw new Error(t('draftErr.attachmentItem'));
    const attachment = item as Record<string, unknown>;
    if (typeof attachment.id !== 'string' || !/^att-[a-zA-Z0-9_-]+$/.test(attachment.id) || attachment.id.length > 100) throw new Error(t('draftErr.attachmentId'));
    if (typeof attachment.name !== 'string' || !attachment.name.trim() || attachment.name.length > 240) throw new Error(t('draftErr.attachmentName'));
    if (typeof attachment.mediaType !== 'string' || !attachment.mediaType || attachment.mediaType.length > 100) throw new Error(t('draftErr.attachmentType'));
    if (typeof attachment.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(attachment.sha256)) throw new Error(t('draftErr.attachmentSha'));
    if (typeof attachment.size !== 'number' || !Number.isSafeInteger(attachment.size) || attachment.size < 0 || attachment.size > MAX_ATTACHMENT_BYTES) throw new Error(t('draftErr.attachmentSize'));
    total += attachment.size;
    if (total > MAX_ATTACHMENT_BYTES) throw new Error(t('draftErr.attachmentTotal'));
    attachments.push({ id: attachment.id, name: attachment.name, size: attachment.size, mediaType: attachment.mediaType, sha256: attachment.sha256 });
  }
  return attachments;
}

export function validateDraft(raw: unknown): MailDraft {
  if (!raw || typeof raw !== 'object') throw new Error(t('draftErr.invalidFormat'));
  const value = raw as Record<string, unknown>;
  const fields = { id: 100, accountId: 200, to: 1000, subject: 1000, bodyText: 50000, updatedAt: 64 };
  for (const [key, limit] of Object.entries(fields)) {
    if (typeof value[key] !== 'string' || value[key].length > limit) throw new Error(t('draftErr.fieldTooLong'));
  }
  if (!/^draft-[a-zA-Z0-9_-]+$/.test(value.id as string) || !value.accountId || !Number.isFinite(Date.parse(value.updatedAt as string))) throw new Error(t('draftErr.invalidIdentity'));
  for (const field of ['cc', 'bcc']) {
    if (value[field] !== undefined && (typeof value[field] !== 'string' || value[field].length > 1000)) throw new Error(t('draftErr.ccBccInvalid'));
  }
  if (value.replyToId !== undefined && (typeof value.replyToId !== 'string' || value.replyToId.length === 0 || value.replyToId.length > 200)) throw new Error(t('draftErr.invalidReply'));
  // HTML 正文可选；上限与 Rust 侧对称（2_000_000 UTF-16 单元，容纳签名 / 内联图片的 base64）。
  if (value.bodyHtml !== undefined && (typeof value.bodyHtml !== 'string' || value.bodyHtml.length > 2_000_000)) throw new Error(t('draftErr.bodyHtmlInvalid'));
  const attachments = validateAttachments(value.attachments);
  const revision = value.revision ?? 0;
  if (typeof revision !== 'number' || !Number.isSafeInteger(revision) || revision < 0 || revision > 1_000_000_000) throw new Error(t('draftErr.invalidRevision'));
  return {
    id: value.id as string, accountId: value.accountId as string, to: value.to as string,
    ...(value.cc ? { cc: value.cc as string } : {}),
    ...(value.bcc ? { bcc: value.bcc as string } : {}),
    subject: value.subject as string, bodyText: value.bodyText as string, updatedAt: value.updatedAt as string, revision,
    ...(value.bodyHtml ? { bodyHtml: value.bodyHtml as string } : {}),
    ...(value.replyToId ? { replyToId: value.replyToId as string } : {}),
    ...(attachments.length ? { attachments } : {}),
  };
}

export function draftAsMessage(draft: MailDraft, accounts: MailAccount[]): MailMessage {
  const account = accounts.find(item => item.id === draft.accountId);
  if (!account) throw new Error(t('draftErr.missingAccount'));
  return {
    id: draft.id, accountId: draft.accountId, threadId: draft.replyToId ?? draft.id, mailbox: 'drafts',
    from: { name: account.name, email: account.email }, to: draft.to ? [draft.to] : [], subject: draft.subject || t('outbox.untitledDraft'),
    preview: draft.bodyText.slice(0, 120), bodyText: draft.bodyText, receivedAt: draft.updatedAt,
    unread: false, starred: false, labelIds: [], draft: { ...draft },
    ...(draft.cc ? { cc: draft.cc.split(',').map(address => address.trim()).filter(Boolean) } : {}),
  };
}
