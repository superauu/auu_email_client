import type { MailGateway } from '../../domain/mail';
import { demoWorkspace } from './fixtures';
import { draftAsMessage, loadDraftMessages, persistDraft } from './drafts';

export const demoMailGateway: MailGateway = {
  mode: 'demo',
  async loadWorkspace(signal) {
    signal?.throwIfAborted();
    const snapshot = structuredClone(demoWorkspace);
    snapshot.messages.push(...loadDraftMessages(demoWorkspace.accounts));
    return snapshot;
  },
  async saveDraft(draft) {
    persistDraft(draft);
    return draftAsMessage(draft, demoWorkspace.accounts);
  },
};
