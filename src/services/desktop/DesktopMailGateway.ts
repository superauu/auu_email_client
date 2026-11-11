import type { MailGateway, MailMessage } from '../../domain/mail';
import { desktopMail, isDesktop } from '../../platform/desktop';
import { demoWorkspace } from '../demo/fixtures';
import { loadLegacyDemoDrafts } from '../demo/drafts';

const PAGE_LIMIT = 100;
/** Single write path into the OS-backed store; all reads flow through the paged query. */
export const desktopMailGateway: MailGateway = {
  mode: 'desktop-local',
  async requestBody(accountId, messageId, revision) { return desktopMail.requestBody(accountId, messageId, revision); },
  async exportAttachment(accountId, messageId, index) { return desktopMail.exportAttachment(accountId, messageId, index); },
  async requestRecovery(accountId, messageId, revision, action) { return desktopMail.requestRecovery(accountId, messageId, revision, action); },
  async loadWorkspace(signal) {
    if (!isDesktop()) throw new Error('本机邮件存储仅在桌面版可用。');
    signal?.throwIfAborted();
    await desktopMail.seed(demoWorkspace);
    signal?.throwIfAborted();
    await desktopMail.importLegacyDrafts(loadLegacyDemoDrafts());
    const base = await desktopMail.workspace();
    const messages: MailMessage[] = [];
    let cursor: string | null = null;
    do {
      signal?.throwIfAborted();
      const page = await desktopMail.page({ cursor, limit: PAGE_LIMIT });
      messages.push(...page.items);
      cursor = page.nextCursor;
    } while (cursor);
    messages.push(...await desktopMail.drafts());
    const syncStates: Record<string, string> = {};
    for (const state of base.syncStates) {
      const current = syncStates[state.accountId] ?? '';
      if (state.updatedAt > current) syncStates[state.accountId] = state.updatedAt;
    }
    return { accounts: base.accounts, folders: base.folders ?? [], labels: base.labels, events: base.events, messages, syncStates };
  },
  async saveDraft(draft, options) {
    return desktopMail.saveDraft(draft, options);
  },
  async updateMessage(accountId, messageId, patch) { return desktopMail.updateMessage(accountId, messageId, patch); },
  async keepMessageLocal(accountId, messageId) { return desktopMail.keepMessageLocal(accountId, messageId); },
};
