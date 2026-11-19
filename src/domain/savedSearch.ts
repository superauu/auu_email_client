import type { MailboxKind, MailQuery } from './mail';
import { t } from '../i18n/core';

/**
 * 保存的搜索：当前查询条件（搜索词 + 范围筛选）的命名快照，纯 UI 偏好
 * （localStorage，与固定入口同级）——不含邮件数据，桌面与演示通用。
 * 应用时对失效引用（账号 / 目录 / 标签已被删除）就地重置，不猜测归属。
 */
export const SAVED_SEARCHES_KEY = 'auumail.ui.searches.v1';
export const MAX_SAVED_SEARCHES = 30;
export const MAX_SAVED_SEARCH_NAME_CHARS = 64;
const MAILBOX_KINDS: MailboxKind[] = ['inbox', 'starred', 'sent', 'drafts', 'archive', 'trash', 'other'];

export interface SavedSearch {
  id: string;
  name: string;
  query: SavedQuery;
}

export interface SavedQuery {
  accountId: string | null;
  mailbox: MailboxKind;
  folderId: string | null;
  labelId: string | null;
  search: string;
  unreadOnly?: boolean;
  draftSource?: 'local' | 'server' | null;
}

/** 名称 trim 后非空且不超过上限；空串表示合法，否则返回当前语言的错误文案。 */
export function savedSearchNameError(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return t('searches.nameRequired');
  if ([...trimmed].length > MAX_SAVED_SEARCH_NAME_CHARS) return t('searches.nameTooLong');
  return '';
}

/** 捕获当前查询为可持久化快照（只取结构化字段，丢弃运行态）。 */
export function captureQuery(query: MailQuery): SavedQuery {
  return {
    accountId: query.accountId,
    mailbox: query.mailbox,
    folderId: query.folderId ?? null,
    labelId: query.labelId ?? null,
    search: query.search,
    ...(query.unreadOnly ? { unreadOnly: true } : {}),
    ...(query.draftSource ? { draftSource: query.draftSource } : {}),
  };
}

/** 校验并归一化持久化条目；形状非法返回 null（坏条目丢弃，不抛错）。 */
export function parseSavedSearch(raw: unknown): SavedSearch | null {
  if (!raw || typeof raw !== 'object') return null;
  const value = raw as Record<string, unknown>;
  const { id, name, query } = value as { id?: unknown; name?: unknown; query?: unknown };
  if (typeof id !== 'string' || !id || typeof name !== 'string' || !name.trim() || !query || typeof query !== 'object') return null;
  const q = query as Record<string, unknown>;
  if (typeof q.mailbox !== 'string' || !MAILBOX_KINDS.includes(q.mailbox as MailboxKind)) return null;
  const text = (field: unknown): string | null => typeof field === 'string' && field ? field : null;
  return {
    id,
    name: name.trim(),
    query: {
      accountId: text(q.accountId),
      mailbox: q.mailbox as MailboxKind,
      folderId: text(q.folderId),
      labelId: text(q.labelId),
      search: typeof q.search === 'string' ? q.search : '',
      ...(q.unreadOnly === true ? { unreadOnly: true } : {}),
      ...(q.draftSource === 'local' || q.draftSource === 'server' ? { draftSource: q.draftSource } : {}),
    },
  };
}

export function parseSavedSearches(raw: unknown): SavedSearch[] {
  if (!Array.isArray(raw)) return [];
  return raw.map(parseSavedSearch).filter((item): item is SavedSearch => item !== null).slice(0, MAX_SAVED_SEARCHES);
}

export interface WorkspaceShape {
  accountIds: Set<string>;
  folders: { id: string; accountId: string }[];
  labelIds: Set<string>;
}

/** 应用前清洗失效引用：账号 / 目录 / 标签已不存在时重置（目录需与账号匹配）。 */
export function sanitizeSavedQuery(saved: SavedQuery, shape: WorkspaceShape): SavedQuery {
  const accountId = saved.accountId && shape.accountIds.has(saved.accountId) ? saved.accountId : null;
  const folderId = saved.folderId && shape.folders.some(folder => folder.id === saved.folderId && (!saved.accountId || folder.accountId === saved.accountId)) ? saved.folderId : null;
  const labelId = saved.labelId && shape.labelIds.has(saved.labelId) ? saved.labelId : null;
  return { ...saved, accountId, folderId, labelId };
}

/** 当前查询与保存条目是否一致（用于侧栏高亮；运行态字段不计）。 */
export function sameQuery(current: MailQuery, saved: SavedQuery): boolean {
  return current.accountId === saved.accountId && current.mailbox === saved.mailbox
    && (current.folderId ?? null) === saved.folderId && (current.labelId ?? null) === saved.labelId
    && current.search === saved.search && !!current.unreadOnly === !!saved.unreadOnly
    && (current.draftSource ?? null) === (saved.draftSource ?? null);
}
