import { create } from 'zustand';
import { desktopMail, isDesktop } from '../platform/desktop';
import { isSyncActive, type SyncJob, type SyncQueueSnapshot } from '../domain/sync';
import type { OutboxEntry } from '../domain/outbox';
import { errorMessage } from '../domain/errors';
import { useWorkspaceStore } from './workspaceStore';
import { t } from '../i18n/core';

interface SyncTransport {
  available: () => boolean;
  enqueue: (ids: string[]) => Promise<SyncQueueSnapshot>;
  read: () => Promise<SyncQueueSnapshot>;
  cancel: (id: string) => Promise<SyncQueueSnapshot>;
  refreshWorkspace: () => Promise<void>;
  /** Outbox 队列（docs/SENDING.md）；浏览器 harness 可缺省。 */
  outboxList?: () => Promise<OutboxEntry[]>;
  cancelOutbox?: (id: string) => Promise<OutboxEntry>;
  returnOutboxDraft?: (id: string) => Promise<OutboxEntry>;
  resolveOutbox?: (id: string, action: 'keep-local' | 'retry') => Promise<OutboxEntry>;
  retryOutbox?: (id: string) => Promise<OutboxEntry>;
}
interface SyncState {
  revision: number;
  jobs: SyncJob[];
  submitting: string[];
  outbox: OutboxEntry[];
  outboxLoaded: boolean;
  outboxPending: string[];
  error: string | null;
  enqueue: (ids: string[]) => Promise<void>;
  refresh: () => Promise<void>;
  cancel: (jobId: string) => Promise<void>;
  outboxCancel: (id: string) => Promise<boolean>;
  outboxReturnDraft: (id: string) => Promise<boolean>;
  outboxResolve: (id: string, action: 'keep-local' | 'retry') => Promise<boolean>;
  outboxRetry: (id: string) => Promise<boolean>;
  monitor: () => () => void;
}

/** 用户手动发起同步的账号（本轮终态消费一次）：完成提示只对手动 / 有结果的轮次弹出，
 *  IDLE 监听续跑与自动空轮次静默（SYNCHRONIZATION.md）。 */
const manualAccounts = new Set<string>();
export function consumeManualSync(accountId: string): boolean {
  return manualAccounts.delete(accountId);
}

/** UI observes the native queue; closing Settings never owns or cancels a task. */
export function createSyncStore(transport: SyncTransport, pollMs = 750, idlePollMs = 5000) {
  let monitors = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let reading: Promise<void> | null = null;
  let needsWorkspaceRefresh = false;
  let refreshingWorkspace = false;
  let pollingFailed = false;
  let outboxEpoch = 0;
  const acknowledged = new Set<string>();
  /** 每账号上一轮 outcome.folders：远端目录结构变化（0 拉取轮次）也要让界面跟进。 */
  const lastFolders = new Map<string, number>();
  const store = create<SyncState>((set, get) => {
    async function changeOutbox(id: string, action: ((id: string) => Promise<OutboxEntry>) | undefined, remove = false) {
      if (!transport.available() || !action || get().outboxPending.includes(id)) return false;
      outboxEpoch += 1;
      set(state => ({ outboxPending: [...state.outboxPending, id], error: null }));
      try {
        const entry = await action(id);
        outboxEpoch += 1;
        set(state => ({ outbox: remove ? state.outbox.filter(item => item.id !== id) : state.outbox.map(item => item.id === id ? entry : item) }));
        if (remove) { needsWorkspaceRefresh = true; await refreshWorkspace(); }
        return true;
      } catch (error) {
        set({ error: t('sync.errOutboxAction', { error: errorMessage(error) }) });
        return false;
      } finally {
        outboxEpoch += 1;
        set(state => ({ outboxPending: state.outboxPending.filter(item => item !== id) }));
        void get().refresh();
      }
    }
    async function refreshWorkspace() {
      if (refreshingWorkspace) return;
      refreshingWorkspace = true;
      try {
        while (needsWorkspaceRefresh) { needsWorkspaceRefresh = false; await transport.refreshWorkspace(); }
        if (get().error?.startsWith(t('sync.errRefreshFailedPrefix'))) set({ error: null });
      } catch (error) { needsWorkspaceRefresh = true; set({ error: `${t('sync.errRefreshFailedPrefix')}${errorMessage(error)}` }); }
      finally { refreshingWorkspace = false; }
    }
    function apply(snapshot: SyncQueueSnapshot) {
      if (snapshot.revision < get().revision) return;
      // A first snapshot can contain a job completed while Settings was closed.
      for (const job of snapshot.jobs) {
        const previousFolders = lastFolders.get(job.accountId);
        if (job.outcome) lastFolders.set(job.accountId, job.outcome.folders);
        if (!isSyncActive(job) && !acknowledged.has(job.id)) {
          acknowledged.add(job.id);
          // 静默同步（SYNCHRONIZATION.md）：无拉取、无回写确认且目录结构未变的空轮次
          // 不重建工作区——自动同步 / IDLE 轮次对阅读界面完全不可见；新邮件、EXPUNGE
          // 等回写确认、远端目录变化才触发刷新。
          const foldersChanged = job.outcome !== null && previousFolders !== undefined && previousFolders !== job.outcome.folders;
          if ((job.status !== 'cancelled' || job.startedAt !== null) && (job.outcome?.refreshNeeded !== false || foldersChanged)) needsWorkspaceRefresh = true;
        }
      }
      const retained = new Set(snapshot.jobs.map(job => job.id));
      for (const id of acknowledged) if (!retained.has(id)) acknowledged.delete(id);
      set({ revision: snapshot.revision, jobs: snapshot.jobs });
      if (needsWorkspaceRefresh) void refreshWorkspace();
    }
    function schedule() {
      clearTimeout(timer);
      if (monitors && transport.available()) timer = setTimeout(() => { void get().refresh(); }, get().jobs.some(isSyncActive) || pollingFailed ? pollMs : idlePollMs);
    }
    return {
      revision: -1, jobs: [], submitting: [], outbox: [], outboxLoaded: false, outboxPending: [], error: null,
      async enqueue(ids) {
        if (!transport.available()) { set({ error: t('sync.desktopOnly') }); return; }
        const pending = new Set(get().submitting);
        const unique = [...new Set(ids)].filter(id => !pending.has(id));
        if (!unique.length) return;
        for (const id of unique) manualAccounts.add(id);
        set({ submitting: [...pending, ...unique], error: null });
        try { apply(await transport.enqueue(unique)); }
        catch (error) { set({ error: errorMessage(error) }); }
        finally { set(state => ({ submitting: state.submitting.filter(id => !unique.includes(id)) })); schedule(); }
      },
      refresh() {
        if (!transport.available()) return Promise.resolve();
        if (reading) return reading;
        reading = (async () => {
          try {
            const snapshot = await transport.read();
            pollingFailed = false;
            if (get().error?.startsWith(t('sync.errProgressPrefix'))) set({ error: null });
            apply(snapshot);
            if (transport.outboxList) {
              const epoch = outboxEpoch;
              const outbox = await transport.outboxList();
              if (epoch === outboxEpoch && !get().outboxPending.length) set({ outbox, outboxLoaded: true });
            }
          }
          catch (error) { pollingFailed = true; set({ error: `${t('sync.errProgressPrefix')}${errorMessage(error)}` }); }
          finally { reading = null; schedule(); }
        })();
        return reading;
      },
      async cancel(jobId) {
        if (!transport.available()) return;
        try { apply(await transport.cancel(jobId)); set({ error: null }); }
        catch (error) { set({ error: errorMessage(error) }); }
        finally { schedule(); }
      },
      outboxCancel: id => changeOutbox(id, transport.cancelOutbox, true),
      outboxReturnDraft: id => changeOutbox(id, transport.returnOutboxDraft, true),
      outboxResolve: (id, action) => changeOutbox(id, transport.resolveOutbox ? value => transport.resolveOutbox!(value, action) : undefined),
      outboxRetry: id => changeOutbox(id, transport.retryOutbox),
      monitor() {
        monitors += 1;
        void get().refresh();
        return () => { monitors = Math.max(0, monitors - 1); if (!monitors) clearTimeout(timer); };
      },
    };
  });
  return store;
}

export const useSyncStore = createSyncStore({
  available: isDesktop,
  enqueue: desktopMail.enqueueSync,
  read: desktopMail.syncJobs,
  cancel: desktopMail.cancelSync,
  refreshWorkspace: () => useWorkspaceStore.getState().reload(),
  outboxList: desktopMail.outboxList,
  cancelOutbox: desktopMail.cancelOutbox,
  returnOutboxDraft: desktopMail.returnOutboxDraft,
  resolveOutbox: desktopMail.resolveOutbox,
  retryOutbox: desktopMail.retryOutbox,
});
