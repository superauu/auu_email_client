import type { MailGateway, MailMessage, MailQuery } from '../../domain/mail';
import { desktopMail, isDesktop } from '../../platform/desktop';
import { realWorkspace } from '../../domain/workspace';
import { t } from '../../i18n/core';

const PAGE_LIMIT = 200;
/** Single write path into the OS-backed store; messages load per view, never as one full snapshot. */
export const desktopMailGateway: MailGateway = {
  mode: 'desktop-local',
  async requestBody(accountId, messageId, revision) { return desktopMail.requestBody(accountId, messageId, revision); },
  async messageHtml(accountId, messageId, allowRemoteImages, mode) { return desktopMail.messageHtml(accountId, messageId, allowRemoteImages, mode); },
  async messageHtmlFragment(accountId, messageId) { return desktopMail.messageHtmlFragment(accountId, messageId); },
  async updateEvent(id, completed) { return desktopMail.updateEvent(id, completed); },
  async sendDraft(draftId, revision) { return desktopMail.sendDraft(draftId, revision); },
  async deleteDraft(draftId) { return desktopMail.deleteDraft(draftId); },
  async exportAttachment(accountId, messageId, index) { return desktopMail.exportAttachment(accountId, messageId, index); },
  async requestRecovery(accountId, messageId, revision, action) { return desktopMail.requestRecovery(accountId, messageId, revision, action); },
  /** 按视图分页拉取：本机草稿视图由草稿缓存覆盖（store 不应调用，返回空页兜底），其余走 Rust 分页查询。 */
  async loadView(query: MailQuery, cursor, limit) {
    if (!isDesktop()) throw new Error(t('store.localMailDesktopOnly'));
    if (query.draftSource === 'local') return { items: [], nextCursor: null };
    return desktopMail.page({
      accountId: query.accountId,
      mailbox: query.mailbox,
      folderId: query.folderId ?? null,
      labelId: query.labelId,
      search: query.search || null,
      cursor: cursor ?? null,
      limit: limit ?? PAGE_LIMIT,
    });
  },
  async viewCounts() { return desktopMail.folderCounts(); },
  async uploadDraft(accountId: string, draftId: string, revision: number): Promise<number> { return desktopMail.uploadDraft(accountId, draftId, revision); },
  async contactSuggestions(accountId) { return desktopMail.contactSuggestions(accountId); },
  /**
   * 工作区基座：账号 / 目录 / 标签 / 事件 / 同步状态 + 全量本机草稿（数量少且撰写必需）。
   * 普通邮件不再随启动全量分页灌入内存——按视图懒加载，计数由 mail_folder_counts 聚合。
   */
  async loadWorkspace(signal) {
    if (!isDesktop()) throw new Error(t('store.localMailDesktopOnly'));
    signal?.throwIfAborted();
    const base = await desktopMail.workspace();
    const accounts = base.accounts.filter(account => account.status !== 'demo');
    signal?.throwIfAborted();
    const messages: MailMessage[] = accounts.length ? await desktopMail.drafts() : [];
    signal?.throwIfAborted();
    const syncStates: Record<string, string> = {};
    for (const state of base.syncStates) {
      const current = syncStates[state.accountId] ?? '';
      if (state.updatedAt > current) syncStates[state.accountId] = state.updatedAt;
    }
    return realWorkspace({ accounts, folders: base.folders ?? [], labels: base.labels, events: base.events, messages, syncStates });
  },
  async saveDraft(draft, options) {
    return desktopMail.saveDraft(draft, options);
  },
  async pickAttachments(draftId, current) {
    return desktopMail.pickAttachments(draftId, current);
  },
  async updateMessage(accountId, messageId, patch) { return desktopMail.updateMessage(accountId, messageId, patch); },
  async purgeMessage(accountId, messageId) { return desktopMail.purgeMessage(accountId, messageId); },
  async keepMessageLocal(accountId, messageId) { return desktopMail.keepMessageLocal(accountId, messageId); },
};
