import { t } from '../../i18n/core';
import type { MailMessage, MailQuery, MailboxKind } from '../../domain/mail';

export function selectMessages(messages: MailMessage[], query: MailQuery): MailMessage[] {
  const needle = query.search.trim().toLocaleLowerCase();
  return messages.filter(message => {
    if (query.unreadOnly && !message.unread) return false;
    if (query.accountId && message.accountId !== query.accountId) return false;
    if (query.folderId) {
      if (!message.folderIds?.includes(query.folderId)) return false;
      // Standard account folders honor local archive intent while MOVE is pending;
      // custom physical folders still expose the actual cached membership as evidence.
      if (query.mailbox !== 'other' && query.mailbox !== 'archive' && message.changes?.some(change => change.field === 'archive')) return false;
    } else if (query.labelId) {
      if (message.mailbox === 'trash' || !message.labelIds.includes(query.labelId)) return false;
    } else if (query.mailbox === 'starred' ? !message.starred || message.mailbox === 'trash' : message.mailbox !== query.mailbox) return false;
    if (query.draftSource === 'local' && !message.draft) return false;
    if (query.draftSource === 'server' && message.draft) return false;
    return !needle || [message.subject, message.from.name, message.from.email, message.bodyText].some(value => value.toLocaleLowerCase().includes(needle));
  }).sort((a, b) => b.receivedAt.localeCompare(a.receivedAt));
}

/** 邮箱类型的本地化显示名（每次调用取当前语言）。 */
export function mailboxName(kind: MailboxKind): string {
  const known: Record<MailboxKind, Parameters<typeof t>[0]> = {
    inbox: 'nav.mailbox.inbox', starred: 'nav.mailbox.starred', sent: 'nav.mailbox.sent', drafts: 'nav.mailbox.drafts', archive: 'nav.mailbox.archive', trash: 'nav.mailbox.trash', other: 'nav.mailbox.other',
  };
  return t(known[kind]);
}
