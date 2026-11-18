import test from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import { createSyncStore } from '../src/stores/syncStore';
import { syncJobSummary } from '../src/domain/sync';
import { consumeManualSync } from '../src/stores/syncStore';
import type { SyncJob, SyncQueueSnapshot } from '../src/domain/sync';
import type { OutboxEntry } from '../src/domain/outbox';

const job = (id: string, status: SyncJob['status'], accountId = id): SyncJob => ({
  id, accountId, status, queuedAt: '2026-11-11T00:00:00Z', startedAt: null, finishedAt: null, outcome: null, error: null,
});
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; }); return { promise, resolve, reject }; }
function setup() {
  let snapshot: SyncQueueSnapshot = { revision: 0, jobs: [] };
  let refreshes = 0;
  const transport = { available: () => true, enqueue: async (_ids: string[]) => snapshot, read: async () => snapshot, cancel: async (_id: string) => snapshot, refreshWorkspace: async () => { refreshes += 1; } };
  return { transport, store: createSyncStore(transport, 5), setSnapshot: (next: SyncQueueSnapshot) => { snapshot = next; }, refreshes: () => refreshes };
}

const outboxEntry = (state: OutboxEntry['state'] = 'failed'): OutboxEntry => ({ id: 'ob-1', draftId: 'draft-1', accountId: 'a', messageId: '<once@example.test>', state, attempts: 2, lastError: '服务器拒绝', queuedAt: '2026-11-16T00:00:00Z', updatedAt: '2026-11-16T00:00:00Z', resolvedBy: null });

test('outbox actions deduplicate and a stale poll cannot restore a released task', async () => {
  const release = deferred<OutboxEntry>();
  const poll = deferred<OutboxEntry[]>();
  const started = deferred<void>();
  let reads = 0; let actions = 0; let refreshes = 0;
  const store = createSyncStore({
    available: () => true, enqueue: async () => ({ revision: 0, jobs: [] }), read: async () => ({ revision: 0, jobs: [] }), cancel: async () => ({ revision: 0, jobs: [] }),
    refreshWorkspace: async () => { refreshes += 1; },
    outboxList: async () => { reads += 1; if (reads === 2) { started.resolve(); return poll.promise; } return reads === 1 ? [outboxEntry()] : []; },
    returnOutboxDraft: async () => { actions += 1; return release.promise; },
  });
  await store.getState().refresh();
  const oldPoll = store.getState().refresh(); await started.promise;
  const returning = store.getState().outboxReturnDraft('ob-1');
  assert.deepEqual(store.getState().outboxPending, ['ob-1']);
  assert.equal(await store.getState().outboxReturnDraft('ob-1'), false);
  release.resolve(outboxEntry());
  assert.equal(await returning, true);
  poll.resolve([outboxEntry()]); await oldPoll;
  assert.equal(actions, 1); assert.equal(refreshes, 1);
  assert.deepEqual(store.getState().outbox, []);
  assert.deepEqual(store.getState().outboxPending, []);
});

test('outbox failures keep the task and surface the error; retry preserves the returned message identity', async () => {
  let fail = true;
  let entry = outboxEntry();
  const store = createSyncStore({
    available: () => true, enqueue: async () => ({ revision: 0, jobs: [] }), read: async () => ({ revision: 0, jobs: [] }), cancel: async () => ({ revision: 0, jobs: [] }),
    refreshWorkspace: async () => {}, outboxList: async () => [entry],
    retryOutbox: async () => { if (fail) throw new Error('发送状态已变化'); entry = { ...entry, state: 'queued', lastError: null }; return entry; },
  });
  await store.getState().refresh();
  assert.equal(await store.getState().outboxRetry('ob-1'), false);
  assert.equal(store.getState().outbox[0].state, 'failed');
  assert.match(store.getState().error!, /发送状态已变化/);
  fail = false;
  assert.equal(await store.getState().outboxRetry('ob-1'), true);
  assert.equal(store.getState().outbox[0].messageId, '<once@example.test>');
  assert.equal(store.getState().outbox[0].state, 'queued');
});

test('out-of-order queue responses cannot erase a newer job, and pending submissions deduplicate', async () => {
  const { transport, store } = setup(); const first = deferred<SyncQueueSnapshot>(); const calls: string[][] = [];
  transport.enqueue = async ids => { calls.push(ids); return ids.includes('a') ? first.promise : { revision: 2, jobs: [job('a', 'running'), job('b', 'queued')] }; };
  const pending = store.getState().enqueue(['a', 'a']);
  await store.getState().enqueue(['a']);
  await store.getState().enqueue(['b']);
  first.resolve({ revision: 1, jobs: [job('a', 'queued')] }); await pending;
  assert.deepEqual(calls, [['a'], ['b']]); assert.equal(store.getState().revision, 2);
  assert.equal(store.getState().jobs.length, 2); assert.deepEqual(store.getState().submitting, []);
});

test('a slow progress read cannot overwrite a submission that completed later', async () => {
  const { transport, store } = setup(); const read = deferred<SyncQueueSnapshot>(); transport.read = () => read.promise;
  transport.enqueue = async () => ({ revision: 2, jobs: [job('new', 'running')] });
  const reading = store.getState().refresh(); await store.getState().enqueue(['new']);
  read.resolve({ revision: 1, jobs: [] }); await reading;
  assert.equal(store.getState().jobs[0].id, 'new');
});

test('terminal outcomes refresh workspace once each while another account remains active', async () => {
  const { store, setSnapshot, refreshes } = setup();
  setSnapshot({ revision: 1, jobs: [job('a', 'failed'), job('b', 'running')] }); await store.getState().refresh();
  await store.getState().refresh(); assert.equal(refreshes(), 1); assert.equal(store.getState().jobs[1].status, 'running');
  setSnapshot({ revision: 2, jobs: [job('a', 'failed'), job('b', 'succeeded')] }); await store.getState().refresh();
  assert.equal(refreshes(), 2);
});

test('a completion during an in-flight workspace refresh triggers a second read', async () => {
  const { transport, store, setSnapshot } = setup(); const first = deferred<void>(); let reads = 0;
  transport.refreshWorkspace = async () => { reads += 1; if (reads === 1) await first.promise; };
  setSnapshot({ revision: 1, jobs: [job('a', 'succeeded')] }); await store.getState().refresh();
  setSnapshot({ revision: 2, jobs: [job('a', 'succeeded'), job('b', 'succeeded')] }); await store.getState().refresh();
  assert.equal(reads, 1); first.resolve(); await delay(0); assert.equal(reads, 2);
});

test('polling recovers after a read failure and survives monitor remounts', async () => {
  const { transport, store } = setup(); let reads = 0;
  transport.read = async () => { reads += 1; if (reads === 1) throw new Error('IPC temporarily unavailable'); return { revision: 2, jobs: [job('a', 'succeeded')] }; };
  const stop = store.getState().monitor(); await delay(35); stop();
  assert.equal(store.getState().jobs[0]?.status, 'succeeded'); assert.equal(store.getState().error, null);
  const stopAgain = store.getState().monitor(); await delay(0); stopAgain(); assert.ok(reads >= 3);
});

test('failed workspace refresh keeps jobs and can be retried without rerunning sync', async () => {
  const { transport, store, setSnapshot } = setup(); let failed = true; let enqueues = 0;
  transport.refreshWorkspace = async () => { if (failed) throw new Error('Local read failed'); };
  transport.enqueue = async () => { enqueues += 1; return { revision: 1, jobs: [] }; };
  setSnapshot({ revision: 2, jobs: [job('a', 'succeeded')] }); await store.getState().refresh(); await delay(0);
  assert.match(store.getState().error!, /工作区刷新失败/); assert.equal(store.getState().jobs[0].status, 'succeeded');
  failed = false; await store.getState().refresh(); await delay(0);
  assert.equal(store.getState().error, null); assert.equal(enqueues, 0);
});

test('unavailable browser mode never submits a fake synchronization', async () => {
  const { transport, store } = setup(); transport.available = () => false; let called = false;
  transport.enqueue = async () => { called = true; return { revision: 1, jobs: [] }; };
  await store.getState().enqueue(['a']); assert.equal(called, false); assert.match(store.getState().error!, /桌面版/);
});

test('background jobs are discovered after an idle poll and IDLE-only completions do not reload messages', async () => {
  const {transport,setSnapshot,refreshes}=setup();
  const store=createSyncStore(transport,5,10);
  const stop=store.getState().monitor();
  await delay(5);
  const listening={...job('idle','succeeded'),outcome:{folders:0,fetchedMessages:0,syncedAt:'2026-11-11T00:00:00Z',warnings:[],listening:true,refreshNeeded:false}};
  setSnapshot({revision:1,jobs:[listening]});
  await delay(40);stop();
  assert.equal(store.getState().jobs[0]?.id,'idle');assert.equal(refreshes(),0);
});

test('stopping a running sync refreshes committed batches while cancelling a queued job does not', async () => {
  const {store,setSnapshot,refreshes}=setup();
  setSnapshot({revision:1,jobs:[job('queued','cancelled')]});await store.getState().refresh();assert.equal(refreshes(),0);
  setSnapshot({revision:2,jobs:[job('queued','cancelled'),{...job('running','cancelled'),startedAt:'2026-11-11T00:00:00Z'}]});
  await store.getState().refresh();assert.equal(refreshes(),1);
});

test('a running sync never rebuilds the workspace; the honest terminal outcome decides', async () => {
  const { transport, setSnapshot, refreshes } = setup();
  const store = createSyncStore(transport, 5, 5000);
  setSnapshot({ revision: 1, jobs: [job('a', 'running')] });
  await store.getState().refresh();
  assert.equal(refreshes(), 0, '运行中快照不再触发刷新（静默同步）');
  setSnapshot({ revision: 2, jobs: [{ ...job('b', 'succeeded'), outcome: { folders: 25, fetchedMessages: 2, syncedAt: '2026-11-19T00:00:00Z', warnings: [] } }] });
  await store.getState().refresh();
  assert.equal(refreshes(), 1, '有拉取的终态立即刷新');
});

test('empty sync rounds stay silent while real changes still refresh', async () => {
  const { transport, setSnapshot, refreshes } = setup();
  const store = createSyncStore(transport, 5, 5000);
  const outcome = (overrides: Partial<import('../src/domain/sync').SyncOutcome> = {}) => ({ folders: 25, fetchedMessages: 0, syncedAt: '2026-11-19T00:00:00Z', warnings: [], refreshNeeded: false, ...overrides });
  // 同一账号每轮新任务 id（真实队列语义：job id 是每轮新 uuid，accountId 不变）。
  setSnapshot({ revision: 1, jobs: [{ ...job('r1', 'succeeded', 'acc'), outcome: outcome() }] });
  await store.getState().refresh();
  assert.equal(refreshes(), 0, '空轮次（无拉取 / 无回写 / 目录未变）不重建工作区');
  setSnapshot({ revision: 2, jobs: [{ ...job('r2', 'succeeded', 'acc'), outcome: outcome({ fetchedMessages: 1, refreshNeeded: true }) }] });
  await store.getState().refresh();
  assert.equal(refreshes(), 1, '新邮件到达立即刷新（后端 refreshNeeded 编码 fetched>0）');
  setSnapshot({ revision: 3, jobs: [{ ...job('r3', 'succeeded', 'acc'), outcome: outcome({ refreshNeeded: true }) }] });
  await store.getState().refresh();
  assert.equal(refreshes(), 2, '回写确认（EXPUNGE / MOVE ack）即使 0 拉取也刷新');
  setSnapshot({ revision: 4, jobs: [{ ...job('r4', 'succeeded', 'acc'), outcome: outcome({ folders: 26 }) }] });
  await store.getState().refresh();
  assert.equal(refreshes(), 3, '远端目录结构变化即使 0 拉取也刷新');
  setSnapshot({ revision: 5, jobs: [{ ...job('r5', 'succeeded', 'acc'), outcome: outcome({ folders: 26 }) }] });
  await store.getState().refresh();
  assert.equal(refreshes(), 3, '目录数稳定后恢复静默（26→25 也算结构变化）');
});

test('running listening jobs report listening instead of syncing', () => {
  const base = { id: 'a', accountId: 'a', queuedAt: '2026-11-19T00:00:00Z', startedAt: '2026-11-19T00:00:01Z', finishedAt: null, outcome: null, error: null };
  assert.equal(syncJobSummary({ ...base, status: 'running', listening: true } as SyncJob), '已连接 · 实时监听中…');
  assert.equal(syncJobSummary({ ...base, status: 'running' } as SyncJob), '同步中…');
});

test('manual enqueues are remembered for completion toasts and consumed once', async () => {
  const { store } = setup();
  await store.getState().enqueue(['manual-a']);
  assert.equal(consumeManualSync('manual-a'), true, '手动发起的账号应被标记');
  assert.equal(consumeManualSync('manual-a'), false, '标记消费一次即清除');
  assert.equal(consumeManualSync('manual-b'), false, '未手动发起的账号不标记');
});
