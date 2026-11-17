import type { MailAccount, MailFolder, MailQuery, MailboxKind } from './mail';
import { t } from '../i18n/core';

export interface NavigationTarget {
  accountId: string | null;
  mailbox: MailboxKind;
  folderId?: string | null;
  draftSource?: 'local' | 'server' | null;
}
export interface DirectoryEntry {
  target: NavigationTarget;
  label: string;
  kind: MailboxKind;
  available: boolean;
  description: string;
}
/** 标准远端目录的本地化显示名（每次调用取当前语言）。 */
export function mailboxName(kind: string): string {
  const known: Record<string, Parameters<typeof t>[0]> = {
    inbox: 'nav.mailbox.inbox', sent: 'nav.mailbox.sent', drafts: 'nav.mailbox.serverDrafts', archive: 'nav.mailbox.archive', trash: 'nav.mailbox.trash',
  };
  return Object.hasOwn(known, kind) ? t(known[kind]) : kind;
}
/** 常用快捷入口（每次调用取当前语言；持久化仅保存结构化 target，不保存文案）。 */
export function defaultShortcuts(): DirectoryEntry[] {
  return [
    { target: { accountId: null, mailbox: 'inbox' }, label: t('nav.allInbox'), kind: 'inbox', available: true, description: t('nav.allInboxDesc') },
    { target: { accountId: null, mailbox: 'starred' }, label: t('nav.mailbox.starred'), kind: 'starred', available: true, description: t('nav.allStarredDesc') },
    { target: { accountId: null, mailbox: 'drafts', draftSource: 'local' }, label: t('nav.allLocalDrafts'), kind: 'drafts', available: true, description: t('nav.allLocalDraftsDesc') },
  ];
}
export function navigationKey(target: NavigationTarget): string {
  return JSON.stringify([target.accountId, target.mailbox, target.folderId ?? null, target.draftSource ?? null]);
}
/** 视图加载状态键：不含 unreadOnly（不改变服务端查询，客户端过滤即可），含 search（改变结果集）。 */
export function viewKey(query: Pick<import('./mail').MailQuery, 'accountId' | 'mailbox' | 'folderId' | 'draftSource' | 'labelId' | 'search'>): string {
  return JSON.stringify([query.accountId, query.mailbox, query.folderId ?? null, query.draftSource ?? null, query.labelId, query.search]);
}
/** 侧栏计数的稳定 id（与 Rust folder_counts 的 view 字段一致）。 */
export function viewCountId(target: Pick<NavigationTarget, 'accountId' | 'mailbox' | 'folderId' | 'draftSource'>): string {
  if (target.draftSource === 'local') return 'drafts-local';
  if (target.folderId) return `folder:${target.folderId}`;
  if (target.mailbox === 'starred') return 'starred';
  return `kind:${target.mailbox}`;
}
export function isNavigationActive(query: MailQuery, target: NavigationTarget): boolean {
  return !query.labelId && navigationKey(query) === navigationKey(target);
}
export function directoryLabel(folder: MailFolder, peers: MailFolder[]): string {
  const standard = ['inbox', 'sent', 'drafts', 'archive', 'trash'].includes(folder.kind) ? mailboxName(folder.kind) : undefined;
  return standard ? peers.filter(peer => peer.kind === folder.kind).length > 1 ? `${standard} · ${folder.name}` : standard : folder.name;
}

/** 目录入口的可见性旗标：懒加载后由视图计数提供，演示端由全量缓存计算。 */
export interface DirectoryFlags {
  /** 是否存在服务器草稿（无远端草稿目录时决定"服务器草稿"入口是否展示）。 */
  serverDrafts: boolean;
}

/** Map standard folders once, preserve distinct physical IDs (even when names/kinds collide). */
export function accountDirectories(account: MailAccount, folders: MailFolder[], flags: DirectoryFlags): DirectoryEntry[] {
  const remote = account.status === 'demo' ? [] : folders.filter(folder => folder.accountId === account.id);
  const entries: DirectoryEntry[] = [];
  const logical = (mailbox: MailboxKind, label: string, draftSource?: 'local' | 'server') => entries.push({ target: { accountId: account.id, mailbox, draftSource }, label, kind: mailbox, available: true, description: draftSource === 'local' ? t('nav.localDraftsOnlyDesc') : draftSource === 'server' ? t('nav.serverDraftsDesc') : t('nav.cacheViewDesc') });
  const physical = (folder: MailFolder) => entries.push({
    target: { accountId: account.id, mailbox: ['inbox', 'starred', 'sent', 'drafts', 'archive', 'trash'].includes(folder.kind) ? folder.kind as MailboxKind : 'other', folderId: folder.id, ...(folder.kind === 'drafts' ? { draftSource: 'server' as const } : {}) },
    label: directoryLabel(folder, remote), kind: ['inbox', 'starred', 'sent', 'drafts', 'archive', 'trash'].includes(folder.kind) ? folder.kind as MailboxKind : 'other', available: folder.available,
    description: `${folder.name}${folder.available ? '' : t('nav.unavailableSuffix')}`,
  });
  for (const kind of ['inbox', 'starred', 'sent', 'drafts', 'archive', 'trash'] as const) {
    if (kind === 'starred') { logical(kind, t('nav.mailbox.starred')); continue; }
    if (kind === 'drafts') logical(kind, t('nav.allLocalDrafts'), 'local');
    const matching = remote.filter(folder => folder.kind === kind);
    // Archive is also a local intent view: a pending MOVE must remain visible here
    // before physical folder membership changes. Distinct remote archives remain accessible.
    // 归档与删除到废纸篓都是本机意图视图：待回写的移动仍在入口可见，
    // 多个同名远端目录才各自展示物理副本（与 Rust 端 mailbox 投影一致）。
    if (kind === 'archive' || kind === 'trash') { logical(kind, mailboxName(kind)); if (matching.length > 1) matching.forEach(physical); continue; }
    if (matching.length) matching.forEach(physical);
    else if (kind !== 'drafts') logical(kind, mailboxName(kind));
    else if (flags.serverDrafts) logical(kind, t('nav.mailbox.serverDrafts'), 'server');
  }
  remote.filter(folder => !['inbox', 'starred', 'sent', 'drafts', 'archive', 'trash'].includes(folder.kind)).forEach(physical);
  // 「其他」兜底入口只在账号尚未列出任何远端目录时出现；列出的目录各自有物理入口，
  // 缓存归属全部指向已列出目录（外键约束），因此不再需要按消息嗅探。
  if (!remote.length) logical('other', t('nav.mailbox.other'));
  return entries;
}

export interface NavigationPreferences {
  pinned: NavigationTarget[];
  expanded: string[] | null;
}
export function validateNavigationPreferences(raw: unknown): NavigationPreferences {
  if (!raw || typeof raw !== 'object') throw new Error('Invalid navigation preferences');
  const value = raw as Record<string, unknown>;
  const id = (input: unknown): input is string => typeof input === 'string' && input.length > 0 && input.length <= 2000;
  if (!Array.isArray(value.pinned) || value.pinned.length > 30 || (value.expanded !== null && (!Array.isArray(value.expanded) || value.expanded.length > 100 || !value.expanded.every(id)))) throw new Error('Invalid navigation preferences');
  const pinned = value.pinned.map((item: unknown): NavigationTarget => {
    if (!item || typeof item !== 'object') throw new Error('Invalid shortcut');
    const target = item as Record<string, unknown>;
    if (!id(target.accountId) || !['inbox', 'starred', 'sent', 'drafts', 'archive', 'trash', 'other'].includes(String(target.mailbox)) || (target.folderId != null && !id(target.folderId)) || (target.draftSource != null && !['local', 'server'].includes(String(target.draftSource)))) throw new Error('Invalid shortcut');
    if ((target.draftSource != null && target.mailbox !== 'drafts') || (target.draftSource === 'local' && target.folderId)) throw new Error('Invalid draft shortcut');
    return { accountId: target.accountId, mailbox: target.mailbox as MailboxKind, folderId: target.folderId as string | null | undefined, draftSource: target.draftSource as NavigationTarget['draftSource'] };
  });
  return { pinned: [...new Map(pinned.map(target => [navigationKey(target), target])).values()], expanded: value.expanded as string[] | null };
}
