import { create } from 'zustand';
import type { DraftAttachment, DraftSaveOptions, MailDraft, MailMessage, MailQuery, MessagePatch, RecoveryAction, WorkspaceSnapshot } from '../domain/mail';
import { errorMessage } from '../domain/errors';
import { isDesktop } from '../platform/desktop';
import { demoMailGateway } from '../services/demo/DemoMailGateway';
import { desktopMailGateway } from '../services/desktop/DesktopMailGateway';
import { selectMessages } from '../features/mail/selectors';
import { t } from '../i18n/core';

interface WorkspaceState extends WorkspaceSnapshot {
  status: 'idle' | 'loading' | 'ready' | 'error';
  error: string | null;
  savingIds: string[];
  clearError: () => void;
  query: MailQuery;
  selectedId: string | null;
  view: 'mail' | 'events';
  load: () => Promise<void>;
  reload: () => Promise<void>;
  setQuery: (changes: Partial<MailQuery>) => void;
  selectMessage: (id: string) => void;
  openMessage: (id: string) => void;
  toggleStar: (id: string) => void;
  toggleUnread: (id: string) => void;
  keepMessageLocal: (id: string) => void;
  requestRecovery: (id: string, action: RecoveryAction) => Promise<boolean>;
  requestBody: (id: string) => Promise<boolean>;
  archiveMessage: (id: string) => void;
  deleteMessage: (id: string) => void;
  moveMessage: (id: string, folderId: string) => void;
  /** 批量操作入口：与单封修改共用串行 / 冲突语义。 */
  applyMessagePatch: (id: string, patch: MessagePatch) => Promise<void>;
  exportAttachment: (id: string, index: number) => Promise<boolean>;
  /** Sanitized sandbox srcdoc for rich rendering; rejects when the environment has no verified raw cache. */
  messageHtml: (accountId: string, messageId: string, allowRemoteImages: boolean, mode: 'light' | 'dark') => Promise<string>;
  /** Register a persisted draft into the outbox queue (docs/SENDING.md). */
  messageHtmlFragment: (accountId: string, messageId: string) => Promise<string>;
  sendDraft: (draftId: string, revision: number) => Promise<import('../domain/outbox').OutboxEntry>;
  /** Explicitly discard a local draft (deletes the persisted row). */
  deleteDraft: (draftId: string) => Promise<void>;
  /** 撰写附件选取（Rust 对话框 + 暂存）；演示环境诚实拒绝。 */
  pickAttachments: (draftId: string, current: DraftAttachment[]) => Promise<DraftAttachment[]>;
  toggleEvent: (id: string) => void;
  setView: (view: 'mail' | 'events') => void;
  saveDraft: (draft: MailDraft, options?: DraftSaveOptions) => Promise<MailDraft>;
}

// A single gateway composition point; live providers must not be imported into components.
const gateway = isDesktop() ? desktopMailGateway : demoMailGateway;
let reloadPending: Promise<void> | null = null;
let draftMutations = 0;
let messageMutations = 0;
const messageEpochs = new Map<string, number>();
const pendingMessages = new Map<string, Promise<unknown>>();
/** Serialize each message, computing toggles from the last committed state; failures keep that state. */
function mutateMessage(id: string, patch: (message: MailMessage) => MessagePatch | null, local = false, recovery?: RecoveryAction, body = false) {
  const store = useWorkspaceStore;
  store.setState(state => ({ savingIds: [...new Set([...state.savingIds, id])] }));
  const task = (pendingMessages.get(id) ?? Promise.resolve()).then(async () => {
    const message = store.getState().messages.find(message => message.id === id);
    if (!message || message.draft) return;
    const changes = patch(message);
    if (!changes) return;
    try {
      const saved = body ? await gateway.requestBody(message.accountId, id, message.stateRevision ?? 0) : recovery ? await gateway.requestRecovery(message.accountId, id, message.stateRevision ?? 0, recovery) : local ? await gateway.keepMessageLocal(message.accountId, id) : await gateway.updateMessage(message.accountId, id, changes);
      messageEpochs.set(id, ++messageMutations);
      store.setState(state => {
        const messages = state.messages.map(current => current.id === id && (saved.stateRevision ?? 0) >= (current.stateRevision ?? 0) ? saved : current);
        const visible = selectMessages(messages, state.query);
        const readingMarkedRead = state.query.unreadOnly && state.selectedId === id && changes.unread === false
          && selectMessages(messages.filter(message => message.id === id), { ...state.query, unreadOnly: false }).length > 0;
        return { messages, selectedId: readingMarkedRead || visible.some(m => m.id === state.selectedId) ? state.selectedId : visible[0]?.id ?? null };
      });
      return true;
    } catch (error) { store.setState({ error: errorMessage(error) }); return false; }
  }).finally(() => {
    if (pendingMessages.get(id) === task) { pendingMessages.delete(id); store.setState(state => ({ savingIds: state.savingIds.filter(value => value !== id) })); }
  });
  pendingMessages.set(id, task);
  return task;
}
export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  accounts: [], messages: [], labels: [], events: [], status: 'idle', error: null, savingIds: [], clearError: () => set({ error: null }),
  query: { accountId: null, mailbox: 'inbox', labelId: null, search: '' }, selectedId: null, view: 'mail',
  async exportAttachment(id, index) {
    const message = get().messages.find(message => message.id === id);
    if (!message) return false;
    set({ error: null });
    try { return await gateway.exportAttachment(message.accountId, id, index); }
    catch (error) { set({ error: errorMessage(error) }); return false; }
  },
  async messageHtml(accountId, messageId, allowRemoteImages, mode) {
    if (!gateway.messageHtml) throw new Error(t('store.noRichBody'));
    return gateway.messageHtml(accountId, messageId, allowRemoteImages, mode);
  },
  async messageHtmlFragment(accountId, messageId) {
    if (!gateway.messageHtmlFragment) throw new Error(t('store.noQuoteFragment'));
    return gateway.messageHtmlFragment(accountId, messageId);
  },
  async sendDraft(draftId, revision) {
    if (!gateway.sendDraft) throw new Error(t('store.noSend'));
    set({ error: null });
    return gateway.sendDraft(draftId, revision);
  },
  async deleteDraft(draftId) {
    if (!gateway.deleteDraft) throw new Error(t('store.noDeleteDraft'));
    await gateway.deleteDraft(draftId);
    draftMutations += 1;
    // 放弃后立即从工作区移除该草稿；选中的草稿被删除时回落到列表第一封。
    set(state => {
      const messages = state.messages.filter(m => m.id !== draftId);
      return { messages, selectedId: state.selectedId === draftId ? selectMessages(messages, state.query)[0]?.id ?? null : state.selectedId };
    });
  },
  async pickAttachments(draftId, current) {
    if (!gateway.pickAttachments) throw new Error(t('store.noPickAttachments'));
    return gateway.pickAttachments(draftId, current);
  },
  async load() {
    if (get().status === 'loading' || get().status === 'ready') return;
    try { await get().reload(); } catch { /* Initial loading errors are rendered from store state. */ }
  },
  /** Coalesces background reads without unmounting the workspace or losing concurrent draft saves. */
  reload() {
    if (reloadPending) return reloadPending;
    const wasReady = get().status === 'ready';
    const version = draftMutations;
    const messageVersion = messageMutations;
    set({ ...(!wasReady ? { status: 'loading' as const } : {}), error: null });
    reloadPending = (async () => {
      try {
        const snapshot = await gateway.loadWorkspace();
        const accountIds = new Set(snapshot.accounts.map(account => account.id));
        const merged = new Map(snapshot.messages.map(message => [message.id, message]));
        for (const current of get().messages) {
          const incoming = merged.get(current.id);
          const changedDuringRead = (messageEpochs.get(current.id) ?? 0) > messageVersion;
          if (!current.draft && accountIds.has(current.accountId) &&
            ((incoming && (current.stateRevision ?? 0) > (incoming.stateRevision ?? 0)) ||
              (changedDuringRead && (!incoming || (current.stateRevision ?? 0) >= (incoming.stateRevision ?? 0))))) merged.set(current.id, current);
        }
        snapshot.messages = [...merged.values()];
        if (version !== draftMutations) {
          const accounts = new Set(snapshot.accounts.map(account => account.id));
          const messages = new Map(snapshot.messages.map(message => [message.id, message]));
          for (const current of get().messages) {
            if (current.draft && accounts.has(current.accountId) && (current.draft.revision ?? 0) > (messages.get(current.id)?.draft?.revision ?? -1)) messages.set(current.id, current);
          }
          snapshot.messages = [...messages.values()];
        }
        const query = { ...get().query };
        if (query.accountId && !accountIds.has(query.accountId)) {
          query.accountId = null; query.folderId = null; query.draftSource = null; query.mailbox = 'inbox'; query.labelId = null;
        }
        if (query.labelId && !snapshot.labels.some(label => label.id === query.labelId)) query.labelId = null;
        const visible = selectMessages(snapshot.messages, query);
        const selectedId = visible.some(message => message.id === get().selectedId) ? get().selectedId : visible[0]?.id ?? null;
        set({ ...snapshot, folders: snapshot.folders ?? [], query, selectedId, status: 'ready' });
      } catch (error) { set({ status: wasReady ? 'ready' : 'error', error: errorMessage(error) }); throw error; }
      finally { reloadPending = null; }
    })();
    return reloadPending;
  },
  setQuery(changes) {
    const state = get();
    const query = { ...state.query, ...changes };
    if (changes.folderId === undefined && ('accountId' in changes || 'mailbox' in changes || 'labelId' in changes)) query.folderId = null;
    if (changes.draftSource === undefined && ('accountId' in changes || 'mailbox' in changes || 'folderId' in changes || 'labelId' in changes)) query.draftSource = null;
    const visible = selectMessages(state.messages, query);
    set({ query, view: 'mail', selectedId: visible.some(m => m.id === state.selectedId) ? state.selectedId : visible[0]?.id ?? null });
  },
  selectMessage(id) {
    if (!selectMessages(get().messages, get().query).some(m => m.id === id)) return;
    set({ selectedId: id, view: 'mail' });
    mutateMessage(id, message => message.unread ? { unread: false } : null);
  },
  openMessage(id) {
    const message = get().messages.find(m => m.id === id);
    if (!message) return;
    set({ query: { accountId: message.accountId, mailbox: message.mailbox, labelId: null, search: '' } });
    get().selectMessage(id);
  },
  toggleStar(id) {
    mutateMessage(id, message => ({ starred: !message.starred }));
  },
  toggleUnread(id) { mutateMessage(id, message => ({ unread: !message.unread })); },
  keepMessageLocal(id) { mutateMessage(id, () => ({}), true); },
  async requestRecovery(id, action) { return await mutateMessage(id, () => ({}), false, action) === true; },
  async requestBody(id) { return await mutateMessage(id, () => ({}), false, undefined, true) === true; },
  deleteMessage(id) {
    mutateMessage(id, message => message.mailbox === 'trash' || message.draft ? null : { delete: true });
  },
  async applyMessagePatch(id, patch) { await mutateMessage(id, () => patch); },
  moveMessage(id, folderId) {
    mutateMessage(id, message => message.draft ? null : { moveTo: folderId });
  },
  archiveMessage(id) {
    mutateMessage(id, message => message.mailbox === 'archive' ? null : { archive: true });
  },
  toggleEvent(id) {
    let completed: boolean | undefined;
    set(state => ({ events: state.events.map(e => e.id === id ? (completed = !e.completed, { ...e, completed }) : e) }));
    if (completed !== undefined) void gateway.updateEvent?.(id, completed).catch(error => set({ error: errorMessage(error) }));
  },
  setView(view) { set({ view }); },
  async saveDraft(draft, options) {
    const message = await gateway.saveDraft(draft, options);
    if (!message.draft) throw new Error(t('store.draftSaveIncomplete'));
    draftMutations += 1;
    set(state => state.accounts.some(account => account.id === message.accountId)
      ? { messages: [message, ...state.messages.filter(m => m.id !== message.id)] } : {});
    return message.draft;
  },
}));
