import type { MailAccount, MailFolder, MailMessage, MailQuery, MailboxKind } from './mail';
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
export function isNavigationActive(query: MailQuery, target: NavigationTarget): boolean {
  return !query.labelId && navigationKey(query) === navigationKey(target);
}
export function directoryLabel(folder: MailFolder, peers: MailFolder[]): string {
  const standard = ['inbox', 'sent', 'drafts', 'archive', 'trash'].includes(folder.kind) ? mailboxName(folder.kind) : undefined;
  return standard ? peers.filter(peer => peer.kind === folder.kind).length > 1 ? `${standard} · ${folder.name}` : standard : folder.name;
}

/** Map standard folders once, preserve distinct physical IDs (even when names/kinds collide). */
export function accountDirectories(account: MailAccount, folders: MailFolder[], messages: MailMessage[]): DirectoryEntry[] {
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
    else if (messages.some(message => message.accountId === account.id && message.mailbox === 'drafts' && !message.draft)) logical(kind, t('nav.mailbox.serverDrafts'), 'server');
  }
  remote.filter(folder => !['inbox', 'starred', 'sent', 'drafts', 'archive', 'trash'].includes(folder.kind)).forEach(physical);
  if (!remote.length || messages.some(message => message.accountId === account.id && message.mailbox === 'other' && !message.folderIds?.some(id => remote.some(folder => folder.id === id)))) logical('other', remote.length ? t('nav.otherCached') : t('nav.mailbox.other'));
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
