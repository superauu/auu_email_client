import type { MailGateway, MailMessage } from '../../domain/mail';
import { desktopMail, isDesktop } from '../../platform/desktop';
import { realWorkspace } from '../../domain/workspace';
import { t } from '../../i18n/core';

const PAGE_LIMIT = 200;
/** Single write path into the OS-backed store; all reads flow through the paged query. */
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
  async loadWorkspace(signal) {
    if (!isDesktop()) throw new Error(t('store.localMailDesktopOnly'));
    signal?.throwIfAborted();
    const base = await desktopMail.workspace();
    const accounts = base.accounts.filter(account => account.status !== 'demo');
    // 各账号分页并行读取（页大小 200），缩短大邮箱首屏等待；结果按账号顺序拼接。
    const perAccount = await Promise.all(accounts.map(async account => {
      const items: MailMessage[] = [];
      let cursor: string | null = null;
      do {
        signal?.throwIfAborted();
        const page = await desktopMail.page({ accountId: account.id, cursor, limit: PAGE_LIMIT });
        items.push(...page.items.filter(message => message.accountId === account.id));
        cursor = page.nextCursor;
      } while (cursor);
      return items;
    }));
    const messages: MailMessage[] = perAccount.flat();
    signal?.throwIfAborted();
    if (accounts.length) messages.push(...await desktopMail.drafts());
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
  async keepMessageLocal(accountId, messageId) { return desktopMail.keepMessageLocal(accountId, messageId); },
};
