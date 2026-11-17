import type { MailGateway } from '../../domain/mail';
import { demoWorkspace } from './fixtures';
import { draftAsMessage, loadDraftMessages, persistDraft, deletePersistedDraft } from './drafts';
import { loadDemoMessageChanges, persistDemoMessage } from './messageChanges';
import { loadDemoEvents, persistDemoEvent } from './events';
import { computeViewCounts } from '../../features/mail/viewCounts';
import { isRecipientAddress } from '../../domain/recipients';
import { t } from '../../i18n/core';

export const demoMailGateway: MailGateway = {
  mode: 'demo',
  async requestBody() { throw new Error(t('demo.noRemoteBody')); },
  async exportAttachment() { throw new Error(t('demo.noAttachment')); },
  async pickAttachments() { throw new Error(t('demo.noPickAttachments')); },
  async requestRecovery() { throw new Error(t('demo.noRecovery')); },
  async loadWorkspace(signal) {
    signal?.throwIfAborted();
    const snapshot = structuredClone(demoWorkspace);
    snapshot.messages = loadDemoMessageChanges(snapshot.messages);
    snapshot.messages.push(...loadDraftMessages(demoWorkspace.accounts));
    snapshot.events = loadDemoEvents(snapshot.events);
    return snapshot;
  },
  // 演示环境没有分页后端：视图计数与收件人建议直接由全量快照计算，语义与桌面端一致。
  async viewCounts() { return computeViewCounts(loadDemoMessageChanges(structuredClone(demoWorkspace.messages))); },
  async contactSuggestions(accountId) {
    const contacts = new Map<string, { name: string; email: string }>();
    for (const message of loadDemoMessageChanges(structuredClone(demoWorkspace.messages)).filter(item => item.accountId === accountId && !item.draft)) {
      for (const contact of [message.from, ...message.to.map(email => ({ name: '', email })), ...(message.cc ?? message.replyTo ?? []).map(email => ({ name: '', email }))]) {
        if (isRecipientAddress(contact.email) && !contacts.has(contact.email.toLowerCase())) contacts.set(contact.email.toLowerCase(), contact);
      }
    }
    return [...contacts.values()];
  },
  async saveDraft(draft, options) {
    return draftAsMessage(persistDraft(draft, options), demoWorkspace.accounts);
  },
  async deleteDraft(draftId) { deletePersistedDraft(draftId); },
  async updateMessage(accountId, id, patch) { return persistDemoMessage(accountId, id, patch); },
  async updateEvent(id, completed) { persistDemoEvent(id, completed); },
  async keepMessageLocal(accountId, id) { return persistDemoMessage(accountId, id, {}); },
};
