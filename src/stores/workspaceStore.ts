import { create } from 'zustand';
import type { MailDraft, MailQuery, WorkspaceSnapshot } from '../domain/mail';
import { errorMessage } from '../domain/errors';
import { demoMailGateway } from '../services/demo/DemoMailGateway';
import { selectMessages } from '../features/mail/selectors';

interface WorkspaceState extends WorkspaceSnapshot {
  status: 'idle' | 'loading' | 'ready' | 'error';
  error: string | null;
  query: MailQuery;
  selectedId: string | null;
  view: 'mail' | 'events';
  load: () => Promise<void>;
  setQuery: (changes: Partial<MailQuery>) => void;
  selectMessage: (id: string) => void;
  openMessage: (id: string) => void;
  toggleStar: (id: string) => void;
  archiveMessage: (id: string) => void;
  toggleEvent: (id: string) => void;
  setView: (view: 'mail' | 'events') => void;
  saveDraft: (draft: MailDraft) => Promise<void>;
}

// A single gateway composition point; live providers must not be imported into components.
const gateway = demoMailGateway;
export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  accounts: [], messages: [], labels: [], events: [], status: 'idle', error: null,
  query: { accountId: null, mailbox: 'inbox', labelId: null, search: '' }, selectedId: null, view: 'mail',
  async load() {
    if (get().status === 'loading' || get().status === 'ready') return;
    set({ status: 'loading', error: null });
    try {
      const snapshot = await gateway.loadWorkspace();
      const selectedId = selectMessages(snapshot.messages, get().query)[0]?.id ?? null;
      set({ ...snapshot, selectedId, status: 'ready' });
    } catch (error) { set({ status: 'error', error: errorMessage(error) }); }
  },
  setQuery(changes) {
    const state = get();
    const query = { ...state.query, ...changes };
    const visible = selectMessages(state.messages, query);
    set({ query, view: 'mail', selectedId: visible.some(m => m.id === state.selectedId) ? state.selectedId : visible[0]?.id ?? null });
  },
  selectMessage(id) {
    if (!selectMessages(get().messages, get().query).some(m => m.id === id)) return;
    set(state => ({ selectedId: id, view: 'mail', messages: state.messages.map(m => m.id === id ? { ...m, unread: false } : m) }));
  },
  openMessage(id) {
    const message = get().messages.find(m => m.id === id);
    if (!message) return;
    set({ query: { accountId: message.accountId, mailbox: message.mailbox, labelId: null, search: '' } });
    get().selectMessage(id);
  },
  toggleStar(id) {
    const state = get();
    const messages = state.messages.map(m => m.id === id ? { ...m, starred: !m.starred } : m);
    const visible = selectMessages(messages, state.query);
    set({ messages, selectedId: visible.some(m => m.id === state.selectedId) ? state.selectedId : visible[0]?.id ?? null });
  },
  archiveMessage(id) {
    const state = get();
    const messages = state.messages.map(m => m.id === id ? { ...m, mailbox: 'archive' as const } : m);
    const visible = selectMessages(messages, state.query);
    set({ messages, selectedId: visible.some(m => m.id === state.selectedId) ? state.selectedId : visible[0]?.id ?? null });
  },
  toggleEvent(id) { set(state => ({ events: state.events.map(e => e.id === id ? { ...e, completed: !e.completed } : e) })); },
  setView(view) { set({ view }); },
  async saveDraft(draft) {
    const message = await gateway.saveDraft(draft);
    set(state => ({ messages: [message, ...state.messages.filter(m => m.id !== message.id)] }));
  },
}));
