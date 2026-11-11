import test from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import { createSyncStore } from '../src/stores/syncStore';
import type { SyncJob, SyncQueueSnapshot } from '../src/domain/sync';

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
