import { create } from 'zustand';
import { desktopMail, isDesktop } from '../platform/desktop';
import { isSyncActive, type SyncJob, type SyncQueueSnapshot } from '../domain/sync';
import { errorMessage } from '../domain/errors';
import { useWorkspaceStore } from './workspaceStore';

interface SyncTransport {
  available: () => boolean;
  enqueue: (ids: string[]) => Promise<SyncQueueSnapshot>;
  read: () => Promise<SyncQueueSnapshot>;
  cancel: (id: string) => Promise<SyncQueueSnapshot>;
  refreshWorkspace: () => Promise<void>;
}
interface SyncState {
  revision: number;
  jobs: SyncJob[];
  submitting: string[];
  error: string | null;
  enqueue: (ids: string[]) => Promise<void>;
  refresh: () => Promise<void>;
  cancel: (jobId: string) => Promise<void>;
  monitor: () => () => void;
}

/** UI observes the native queue; closing Settings never owns or cancels a task. */
export function createSyncStore(transport: SyncTransport, pollMs = 750, idlePollMs = 5000) {
  let monitors = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let reading: Promise<void> | null = null;
  let needsWorkspaceRefresh = false;
  let refreshingWorkspace = false;
  let pollingFailed = false;
  const acknowledged = new Set<string>();
  const store = create<SyncState>((set, get) => {
    async function refreshWorkspace() {
      if (refreshingWorkspace) return;
      refreshingWorkspace = true;
      try {
        while (needsWorkspaceRefresh) { needsWorkspaceRefresh = false; await transport.refreshWorkspace(); }
        if (get().error?.startsWith('同步结果已保存，但工作区刷新失败：')) set({ error: null });
      } catch (error) { needsWorkspaceRefresh = true; set({ error: `同步结果已保存，但工作区刷新失败：${errorMessage(error)}` }); }
      finally { refreshingWorkspace = false; }
    }
    function apply(snapshot: SyncQueueSnapshot) {
      if (snapshot.revision < get().revision) return;
      // A first snapshot can contain a job completed while Settings was closed.
      for (const job of snapshot.jobs) {
        if (!isSyncActive(job) && !acknowledged.has(job.id)) {
          acknowledged.add(job.id);
          if ((job.status !== 'cancelled' || job.startedAt !== null) && job.outcome?.refreshNeeded !== false) needsWorkspaceRefresh = true;
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
      revision: -1, jobs: [], submitting: [], error: null,
      async enqueue(ids) {
        if (!transport.available()) { set({ error: '请在桌面版中同步真实邮箱。' }); return; }
        const pending = new Set(get().submitting);
        const unique = [...new Set(ids)].filter(id => !pending.has(id));
        if (!unique.length) return;
        set({ submitting: [...pending, ...unique], error: null });
        try { apply(await transport.enqueue(unique)); }
        catch (error) { set({ error: errorMessage(error) }); }
        finally { set(state => ({ submitting: state.submitting.filter(id => !unique.includes(id)) })); schedule(); }
      },
      refresh() {
        if (!transport.available()) return Promise.resolve();
        if (reading) return reading;
        reading = (async () => {
          try { const snapshot = await transport.read(); pollingFailed = false; if (get().error?.startsWith('无法读取同步进度：')) set({ error: null }); apply(snapshot); }
          catch (error) { pollingFailed = true; set({ error: `无法读取同步进度：${errorMessage(error)}` }); }
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
});
