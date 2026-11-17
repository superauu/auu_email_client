import type { MailboxKind, MailMessage, ViewCountEntry } from '../../domain/mail';
import { viewCountId } from '../../domain/navigation';

/**
 * 读取一个视图的计数。账号为空（跨账号快捷入口）时对所有账号求和；
 * 计数尚未加载返回 null（侧栏此时不显示数字，避免误报 0）。
 */
export function countForView(
  counts: ViewCountEntry[] | null,
  target: { accountId: string | null; mailbox: MailboxKind; folderId?: string | null; draftSource?: 'local' | 'server' | null },
): ViewCountEntry | null {
  if (!counts) return null;
  const id = viewCountId(target);
  const rows = counts.filter(entry => entry.view === id && (!target.accountId || entry.accountId === target.accountId));
  return rows.reduce<ViewCountEntry>(
    (sum, row) => ({ accountId: target.accountId ?? '', view: id, total: sum.total + row.total, unread: sum.unread + row.unread }),
    { accountId: target.accountId ?? '', view: id, total: 0, unread: 0 },
  );
}

/**
 * 演示环境的视图计数：由全量快照直接计算，语义与桌面端 Rust folder_counts 一致
 * （kind 投影 + 星标排除废纸篓 + 本机草稿单独计数）。
 */
export function computeViewCounts(messages: MailMessage[]): ViewCountEntry[] {
  const counts: ViewCountEntry[] = [];
  for (const accountId of new Set(messages.map(message => message.accountId))) {
    const mine = messages.filter(message => message.accountId === accountId);
    // 本机草稿不在消息表内：kind 视图一律排除带 draft 元数据的行（镜像 Rust 计数语义）。
    const regular = mine.filter(message => !message.draft);
    for (const kind of ['inbox', 'sent', 'drafts', 'archive', 'trash', 'other'] as const) {
      const rows = regular.filter(message => message.mailbox === kind);
      if (rows.length) counts.push({ accountId, view: `kind:${kind}`, total: rows.length, unread: rows.filter(message => message.unread).length });
    }
    const starredRows = regular.filter(message => message.starred && message.mailbox !== 'trash');
    if (starredRows.length) counts.push({ accountId, view: 'starred', total: starredRows.length, unread: starredRows.filter(message => message.unread).length });
    const localDrafts = mine.filter(message => message.draft);
    if (localDrafts.length) counts.push({ accountId, view: 'drafts-local', total: localDrafts.length, unread: 0 });
  }
  return counts;
}
