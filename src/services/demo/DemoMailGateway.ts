import type { MailGateway } from '../../domain/mail';
import { demoWorkspace } from './fixtures';
import { draftAsMessage, loadDraftMessages, persistDraft, deletePersistedDraft } from './drafts';
import { loadDemoMessageChanges, persistDemoMessage } from './messageChanges';
import { loadDemoEvents, persistDemoEvent } from './events';
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
  async saveDraft(draft, options) {
    return draftAsMessage(persistDraft(draft, options), demoWorkspace.accounts);
  },
  async deleteDraft(draftId) { deletePersistedDraft(draftId); },
  async updateMessage(accountId, id, patch) { return persistDemoMessage(accountId, id, patch); },
  async updateEvent(id, completed) { persistDemoEvent(id, completed); },
  async keepMessageLocal(accountId, id) { return persistDemoMessage(accountId, id, {}); },
};
