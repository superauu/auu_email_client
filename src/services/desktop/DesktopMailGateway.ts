import type { MailGateway, MailMessage } from '../../domain/mail';
import { desktopMail, isDesktop } from '../../platform/desktop';
import { demoWorkspace } from '../demo/fixtures';
import { loadDraftMessages, persistDraft, draftAsMessage } from '../demo/drafts';

const PAGE_LIMIT = 100;
/** Single write path into the OS-backed store; all reads flow through the paged query. */
export const desktopMailGateway: MailGateway = {
  mode: 'desktop-local',
  async loadWorkspace(signal) {
    if (!isDesktop()) throw new Error('本机邮件存储仅在桌面版可用。');
    signal?.throwIfAborted();
    await desktopMail.seed(demoWorkspace);
    const base = await desktopMail.workspace();
    const messages: MailMessage[] = [];
    let cursor: string | null = null;
    do {
      const page = await desktopMail.page({ cursor, limit: PAGE_LIMIT });
      messages.push(...page.items);
      cursor = page.nextCursor;
    } while (cursor);
    messages.push(...loadDraftMessages(base.accounts));
    return { ...base, messages };
  },
  async saveDraft(draft) {
    persistDraft(draft);
    return draftAsMessage(draft, demoWorkspace.accounts);
  },
};
