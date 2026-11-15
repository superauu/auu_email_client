import type { MailAccount, MailFolder, MailMessage, MailQuery, MailboxKind } from './mail';

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
const names: Record<string, string> = { inbox: '收件箱', sent: '已发送', drafts: '服务器草稿', archive: '归档', trash: '废纸篓' };
export const defaultShortcuts: DirectoryEntry[] = [
  { target: { accountId: null, mailbox: 'inbox' }, label: '全部收件箱', kind: 'inbox', available: true, description: '所有账号的收件箱' },
  { target: { accountId: null, mailbox: 'starred' }, label: '星标邮件', kind: 'starred', available: true, description: '所有账号的星标邮件' },
  { target: { accountId: null, mailbox: 'drafts', draftSource: 'local' }, label: '本机草稿', kind: 'drafts', available: true, description: '所有账号保存在本机的可编辑草稿' },
];
export function navigationKey(target: NavigationTarget): string {
  return JSON.stringify([target.accountId, target.mailbox, target.folderId ?? null, target.draftSource ?? null]);
}
export function isNavigationActive(query: MailQuery, target: NavigationTarget): boolean {
  return !query.labelId && navigationKey(query) === navigationKey(target);
}
export function directoryLabel(folder: MailFolder, peers: MailFolder[]): string {
  const standard = Object.hasOwn(names, folder.kind) ? names[folder.kind] : undefined;
  return standard ? peers.filter(peer => peer.kind === folder.kind).length > 1 ? `${standard} · ${folder.name}` : standard : folder.name;
}

/** Map standard folders once, preserve distinct physical IDs (even when names/kinds collide). */
export function accountDirectories(account: MailAccount, folders: MailFolder[], messages: MailMessage[]): DirectoryEntry[] {
  const remote = account.status === 'demo' ? [] : folders.filter(folder => folder.accountId === account.id);
  const entries: DirectoryEntry[] = [];
  const logical = (mailbox: MailboxKind, label: string, draftSource?: 'local' | 'server') => entries.push({ target: { accountId: account.id, mailbox, draftSource }, label, kind: mailbox, available: true, description: draftSource === 'local' ? '仅保存在这台设备，可离线编辑' : draftSource === 'server' ? '服务器草稿缓存，编辑时复制为本机草稿' : '此账号的本机缓存视图' });
  const physical = (folder: MailFolder) => entries.push({
    target: { accountId: account.id, mailbox: Object.hasOwn(names, folder.kind) ? folder.kind as MailboxKind : 'other', folderId: folder.id, ...(folder.kind === 'drafts' ? { draftSource: 'server' as const } : {}) },
    label: directoryLabel(folder, remote), kind: Object.hasOwn(names, folder.kind) ? folder.kind as MailboxKind : 'other', available: folder.available,
    description: `${folder.name}${folder.available ? '' : ' · 服务器目录不可用，仍可阅读本机缓存'}`,
  });
  for (const kind of ['inbox', 'starred', 'sent', 'drafts', 'archive', 'trash'] as const) {
    if (kind === 'starred') { logical(kind, '星标邮件'); continue; }
    if (kind === 'drafts') logical(kind, '本机草稿', 'local');
    const matching = remote.filter(folder => folder.kind === kind);
    // Archive is also a local intent view: a pending MOVE must remain visible here
    // before physical folder membership changes. Distinct remote archives remain accessible.
    if (kind === 'archive') { logical(kind, names[kind]); if (matching.length > 1) matching.forEach(physical); continue; }
    if (matching.length) matching.forEach(physical);
    else if (kind !== 'drafts') logical(kind, names[kind]);
    else if (messages.some(message => message.accountId === account.id && message.mailbox === 'drafts' && !message.draft)) logical(kind, '服务器草稿', 'server');
  }
  remote.filter(folder => !(Object.hasOwn(names, folder.kind))).forEach(physical);
  if (!remote.length || messages.some(message => message.accountId === account.id && message.mailbox === 'other' && !message.folderIds?.some(id => remote.some(folder => folder.id === id)))) logical('other', remote.length ? '其他缓存邮件' : '其他邮件');
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
