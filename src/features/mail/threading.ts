import type { MailMessage } from '../../domain/mail';

/**
 * 主题归一化：反复剥离回复 / 转发前缀（中英文与常见欧洲缩写），折叠空白并转小写，
 * 作为「同标题折叠」的会话分组键。空结果返回空串（调用方不折叠空主题）。
 */
export function normalizeSubject(subject: string): string {
  let text = subject.trim();
  for (;;) {
    const next = text.replace(/^\s*(?:re(?:\s*\[\d+\])?|fwd?|aw|sv|回复|答复|转发)\s*[::：]?\s*/i, '');
    if (next === text) break;
    text = next;
    if (!text.trim()) return '';
  }
  return text.replace(/\s+/g, ' ').trim().toLocaleLowerCase();
}

export interface ThreadGroup {
  /** `${accountId}\u{1f}${归一化主题}`：全部收件箱等跨账号视图里，不同账号的同名主题不混。 */
  key: string;
  /** 展示用主题：会话根（最早一封）的实际主题，回复 / 转发链不把前缀带进组标题。 */
  subject: string;
  /** 组内邮件，时间正序（旧 → 新）。 */
  messages: MailMessage[];
}

/** 同标题会话分组：只有 ≥2 封才成组；组按最新邮件时间倒序，与列表排序骨架一致。 */
export function groupBySubject(messages: MailMessage[]): ThreadGroup[] {
  const grouped = new Map<string, MailMessage[]>();
  for (const message of messages) {
    const key = normalizeSubject(message.subject);
    if (!key) continue;
    const id = `${message.accountId}\u{1f}${key}`;
    const list = grouped.get(id);
    if (list) list.push(message);
    else grouped.set(id, [message]);
  }
  const groups: ThreadGroup[] = [];
  for (const [key, list] of grouped) {
    if (list.length < 2) continue;
    list.sort((a, b) => a.receivedAt.localeCompare(b.receivedAt) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    groups.push({ key, subject: list[0].subject, messages: list });
  }
  groups.sort((a, b) => b.messages[b.messages.length - 1].receivedAt.localeCompare(a.messages[a.messages.length - 1].receivedAt));
  return groups;
}

export type ThreadRow =
  | { kind: 'group'; group: ThreadGroup; unread: number; latest: MailMessage; expanded: boolean }
  | { kind: 'message'; message: MailMessage; childOf?: string };

/**
 * 折叠开关开启时的列表行：把 ≥2 封同标题的组折叠为一行（位置在组内最新一封处，与 Outlook
 * 一致），展开的组保留组行（作为折叠入口）并按时间正序输出子行；不成组的消息保持普通行。
 * 纯函数，便于单测。
 */
export function conversationRows(messages: MailMessage[], expandedKeys: ReadonlySet<string>): ThreadRow[] {
  const memberToGroup = new Map<string, ThreadGroup>();
  for (const group of groupBySubject(messages)) {
    for (const message of group.messages) memberToGroup.set(message.id, group);
  }
  const emitted = new Set<string>();
  const rows: ThreadRow[] = [];
  for (const message of messages) {
    const group = memberToGroup.get(message.id);
    if (!group) {
      rows.push({ kind: 'message', message });
      continue;
    }
    if (emitted.has(group.key)) continue;
    emitted.add(group.key);
    const unread = group.messages.filter(item => item.unread).length;
    const latest = group.messages[group.messages.length - 1];
    rows.push({ kind: 'group', group, unread, latest, expanded: expandedKeys.has(group.key) });
    if (expandedKeys.has(group.key)) {
      for (const member of group.messages) rows.push({ kind: 'message', message: member, childOf: group.key });
    }
  }
  return rows;
}
