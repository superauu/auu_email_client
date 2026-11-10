import type { MailMessage, MailQuery } from '../../domain/mail';

export function selectMessages(messages: MailMessage[], query: MailQuery): MailMessage[] {
  const needle = query.search.trim().toLocaleLowerCase();
  return messages.filter(message => {
    if (query.accountId && message.accountId !== query.accountId) return false;
    if (query.labelId) {
      if (message.mailbox === 'trash' || !message.labelIds.includes(query.labelId)) return false;
    } else if (query.mailbox === 'starred' ? !message.starred || message.mailbox === 'trash' : message.mailbox !== query.mailbox) return false;
    return !needle || [message.subject, message.from.name, message.from.email, message.bodyText].some(value => value.toLocaleLowerCase().includes(needle));
  }).sort((a, b) => b.receivedAt.localeCompare(a.receivedAt));
}

export const mailboxNames = {
  inbox: '收件箱', starred: '星标邮件', sent: '已发送', drafts: '草稿箱', archive: '归档', trash: '废纸篓',
} as const;
