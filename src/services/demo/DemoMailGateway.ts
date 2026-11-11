import type { MailGateway } from '../../domain/mail';
import { demoWorkspace } from './fixtures';
import { draftAsMessage, loadDraftMessages, persistDraft } from './drafts';
import { loadDemoMessageChanges, persistDemoMessage } from './messageChanges';

export const demoMailGateway: MailGateway = {
  mode: 'demo',
  async requestBody() { throw new Error('演示邮件没有可下载的远端原文。'); },
  async exportAttachment() { throw new Error('演示邮件没有真实附件，请在桌面版同步 IMAP 邮件。'); },
  async requestRecovery() { throw new Error('演示邮件不提供远端定位核对，请在桌面版使用真实 IMAP 缓存。'); },
  async loadWorkspace(signal) {
    signal?.throwIfAborted();
    const snapshot = structuredClone(demoWorkspace);
    snapshot.messages = loadDemoMessageChanges(snapshot.messages);
    snapshot.messages.push(...loadDraftMessages(demoWorkspace.accounts));
    return snapshot;
  },
  async saveDraft(draft, options) {
    return draftAsMessage(persistDraft(draft, options), demoWorkspace.accounts);
  },
  async updateMessage(accountId, id, patch) { return persistDemoMessage(accountId, id, patch); },
  async keepMessageLocal(accountId, id) { return persistDemoMessage(accountId, id, {}); },
};
