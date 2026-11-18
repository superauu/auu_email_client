import type { MailMessage } from '../../domain/mail';

/**
 * 主题归一化：反复剥离回复 / 转发前缀（中英文与常见欧洲缩写），折叠空白并转小写，
 * 作为会话分组的标题键。空结果返回空串（调用方不折叠空主题）。
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
  /** 会话组键（账号内唯一）：全部收件箱等跨账号视图里，不同账号的会话不混。 */
  key: string;
  /** 展示用主题：会话根（最早一封）的实际主题，回复 / 转发链不把前缀带进组标题。 */
  subject: string;
  /** 组内邮件，时间正序（旧 → 新）。 */
  messages: MailMessage[];
}

/**
 * 会话分组（Outlook「显示为对话」语义）：账号内通过**引用链**（In-Reply-To / References，
 * 覆盖回复中途改主题的会话）或**归一化标题**（无引用头的同标题邮件）连通的消息归入同组，
 * 并查集实现；只有 ≥2 封才成组。组按最新邮件时间倒序，与列表排序骨架一致。
 */
export function groupConversations(messages: MailMessage[]): ThreadGroup[] {
  // 并查集：节点为消息下标。两种连通关系——引用链与同账号同标题。
  const parent = messages.map((_, index) => index);
  const find = (node: number): number => {
    while (parent[node] !== node) {
      parent[node] = parent[parent[node]];
      node = parent[node];
    }
    return node;
  };
  const union = (a: number, b: number) => {
    const [ra, rb] = [find(a), find(b)];
    if (ra !== rb) parent[Math.max(ra, rb)] = Math.min(ra, rb);
  };

  // 1) 引用链：已知 Message-ID → 邮件下标（裸 id 约定，比较统一小写）。
  const byMessageId = new Map<string, number>();
  messages.forEach((message, index) => {
    if (message.headerMessageId) {
      const key = message.headerMessageId.trim().toLowerCase();
      if (!byMessageId.has(key)) byMessageId.set(key, index);
    }
  });
  messages.forEach((message, index) => {
    const chain = (message.headerReferences ?? '')
      .split(/\s+/)
      .map(item => item.trim().toLowerCase())
      .filter(Boolean);
    for (const reference of chain) {
      const known = byMessageId.get(reference);
      if (known !== undefined && known !== index) union(index, known);
    }
  });
  // 2) 同账号 + 归一化标题连通（无引用头的客户端 / 群发通知场景）。
  const bySubject = new Map<string, number>();
  messages.forEach((message, index) => {
    const key = normalizeSubject(message.subject);
    if (!key) return;
    const accountKey = `${message.accountId}\u{1f}${key}`;
    const known = bySubject.get(accountKey);
    if (known === undefined) bySubject.set(accountKey, index);
    else union(index, known);
  });

  const grouped = new Map<number, MailMessage[]>();
  messages.forEach((message, index) => {
    const root = find(index);
    const list = grouped.get(root);
    if (list) list.push(message);
    else grouped.set(root, [message]);
  });
  const groups: ThreadGroup[] = [];
  for (const list of grouped.values()) {
    if (list.length < 2) continue;
    list.sort((a, b) => a.receivedAt.localeCompare(b.receivedAt) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    groups.push({ key: list[0].id, subject: list[0].subject, messages: list });
  }
  groups.sort((a, b) => b.messages[b.messages.length - 1].receivedAt.localeCompare(a.messages[a.messages.length - 1].receivedAt));
  return groups;
}

/** 兼容别名：同标题分组语义并入 groupConversations（标题连通是其中的引用链回退路径）。 */
export const groupBySubject = groupConversations;

export type ThreadRow =
  | { kind: 'group'; group: ThreadGroup; unread: number; latest: MailMessage; expanded: boolean }
  | { kind: 'message'; message: MailMessage; childOf?: string };

/**
 * 折叠开关开启时的列表行：把 ≥2 封同会话的组折叠为一行（位置在组内最新一封处，与 Outlook
 * 一致），展开的组保留组行（作为折叠入口）并按时间正序输出子行；不成组的消息保持普通行。
 * 纯函数，便于单测。
 */
export function conversationRows(messages: MailMessage[], expandedKeys: ReadonlySet<string>): ThreadRow[] {
  const memberToGroup = new Map<string, ThreadGroup>();
  for (const group of groupConversations(messages)) {
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
