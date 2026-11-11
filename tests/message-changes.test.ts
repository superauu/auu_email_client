import test from 'node:test';
import assert from 'node:assert/strict';
import { demoMailGateway } from '../src/services/demo/DemoMailGateway';
import { demoWorkspace } from '../src/services/demo/fixtures';
import { useWorkspaceStore } from '../src/stores/workspaceStore';
import type { MailMessage, WorkspaceSnapshot } from '../src/domain/mail';

const tick = () => new Promise<void>(resolve => setImmediate(resolve));
async function settle() {
  for (let count = 0; count < 100 && useWorkspaceStore.getState().savingIds.length; count++) await tick();
  assert.deepEqual(useWorkspaceStore.getState().savingIds, []);
}
test('demo read, star and archive survive reload offline and failed storage leaves saved state intact', async () => {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
  const original = demoWorkspace.messages[0];
  try {
    await demoMailGateway.updateMessage(original.accountId, original.id, { unread: false, starred: !original.starred, archive: true });
    const saved = (await demoMailGateway.loadWorkspace()).messages.find(message => message.id === original.id)!;
    assert.equal(saved.unread, false); assert.equal(saved.starred, !original.starred); assert.equal(saved.mailbox, 'archive');
    assert.ok(saved.changes?.every(change => change.state === 'local'));
    await assert.rejects(demoMailGateway.updateMessage('another-account', original.id, { starred: original.starred }));
    storage.setItem = () => { throw new Error('Quota'); };
    await assert.rejects(demoMailGateway.updateMessage(original.accountId, original.id, { starred: original.starred }), /未保存/);
    assert.equal((await demoMailGateway.loadWorkspace()).messages.find(message => message.id === original.id)?.starred, saved.starred);
  } finally { Reflect.deleteProperty(globalThis, 'localStorage'); }
});

test('queued toggles use the last saved value and a stale background snapshot cannot erase them', async () => {
  const originalUpdate = demoMailGateway.updateMessage; const originalLoad = demoMailGateway.loadWorkspace;
  const original = { ...structuredClone(demoWorkspace.messages[0]), starred: false };
  const calls: boolean[] = [];
  let releaseSave!: () => void; let releaseLoad!: (value: WorkspaceSnapshot) => void;
  let persisted: MailMessage = original;
  try {
    useWorkspaceStore.setState({ ...structuredClone(demoWorkspace), messages: [original], status: 'ready', savingIds: [], error: null });
    demoMailGateway.loadWorkspace = () => new Promise(resolve => { releaseLoad = resolve; });
    demoMailGateway.updateMessage = async (_account, _id, patch) => {
      calls.push(patch.starred!);
      if (calls.length === 1) await new Promise<void>(resolve => { releaseSave = resolve; });
      persisted = { ...persisted, starred: patch.starred!, stateRevision: (persisted.stateRevision ?? 0) + 1 };
      return persisted;
    };
    const reload = useWorkspaceStore.getState().reload();
    useWorkspaceStore.getState().toggleStar(original.id); useWorkspaceStore.getState().toggleStar(original.id);
    await tick(); assert.deepEqual(calls, [true]);
    releaseSave(); await settle(); assert.deepEqual(calls, [true, false]);
    releaseLoad({ ...structuredClone(demoWorkspace), messages: [{ ...original, starred: true }] }); await reload;
    assert.equal(useWorkspaceStore.getState().messages[0].starred, false);
    assert.equal(useWorkspaceStore.getState().messages[0].stateRevision, 2);
  } finally { demoMailGateway.updateMessage = originalUpdate; demoMailGateway.loadWorkspace = originalLoad; }
});

test('failed local saves keep read, star and folder state and expose an actionable error', async () => {
  const originalUpdate = demoMailGateway.updateMessage;
  const original = { ...structuredClone(demoWorkspace.messages[0]), mailbox: 'inbox' as const, unread: true, starred: false };
  try {
    useWorkspaceStore.setState({ ...structuredClone(demoWorkspace), messages: [original], status: 'ready', savingIds: [], error: null });
    demoMailGateway.updateMessage = async () => { throw new Error('磁盘写入失败'); };
    useWorkspaceStore.getState().selectMessage(original.id); useWorkspaceStore.getState().toggleStar(original.id); useWorkspaceStore.getState().archiveMessage(original.id);
    await settle();
    assert.deepEqual(useWorkspaceStore.getState().messages[0], original);
    assert.match(useWorkspaceStore.getState().error ?? '', /磁盘写入失败/);
    useWorkspaceStore.getState().clearError(); assert.equal(useWorkspaceStore.getState().error, null);
  } finally { demoMailGateway.updateMessage = originalUpdate; }
});

test('a delayed local save response cannot replace a newer server-confirmed workspace version', async () => {
  const originalUpdate = demoMailGateway.updateMessage; const originalLoad = demoMailGateway.loadWorkspace;
  const original = { ...structuredClone(demoWorkspace.messages[0]), starred: false, stateRevision: 0, changes: [] };
  let release!: (message: MailMessage) => void;
  try {
    useWorkspaceStore.setState({ ...structuredClone(demoWorkspace), messages: [original], status: 'ready', savingIds: [], error: null });
    demoMailGateway.updateMessage = () => new Promise(resolve => { release = resolve; });
    useWorkspaceStore.getState().toggleStar(original.id); await tick();
    const confirmed: MailMessage = { ...original, starred: true, stateRevision: 3, changes: [] };
    demoMailGateway.loadWorkspace = async () => ({ ...structuredClone(demoWorkspace), messages: [confirmed] });
    await useWorkspaceStore.getState().reload();
    assert.equal(useWorkspaceStore.getState().messages[0].stateRevision, 3);
    release({ ...original, starred: true, stateRevision: 1, changes: [{ field: 'starred', state: 'queued', error: null }] });
    await settle();
    assert.deepEqual(useWorkspaceStore.getState().messages[0], confirmed);
  } finally { demoMailGateway.updateMessage = originalUpdate; demoMailGateway.loadWorkspace = originalLoad; }
});

test('recovery is serialized after local edits and submits the latest persisted revision', async () => {
  const originalUpdate = demoMailGateway.updateMessage; const originalRecovery = demoMailGateway.requestRecovery;
  const message: MailMessage = { ...structuredClone(demoWorkspace.messages[0]), starred: false, stateRevision: 7, recovery: { state: 'available', detail: '需要核对', location: null, evidence: null } };
  let release!: () => void;
  try {
    useWorkspaceStore.setState({ ...structuredClone(demoWorkspace), messages: [message], status: 'ready', savingIds: [], error: null });
    demoMailGateway.updateMessage = async () => { await new Promise<void>(resolve => { release = resolve; }); return { ...message, starred: true, stateRevision: 8 }; };
    demoMailGateway.requestRecovery = async (account, id, revision, action) => {
      assert.equal(account, message.accountId); assert.equal(id, message.id); assert.equal(revision, 8); assert.equal(action, 'check');
      return { ...message, starred: true, stateRevision: 9, recovery: { state: 'queued', detail: '等待核对', location: null, evidence: null } };
    };
    useWorkspaceStore.getState().toggleStar(message.id); await tick();
    const request = useWorkspaceStore.getState().requestRecovery(message.id, 'check'); release();
    assert.equal(await request, true); await settle();
    assert.equal(useWorkspaceStore.getState().messages[0].recovery?.state, 'queued');
    assert.equal(useWorkspaceStore.getState().messages[0].starred, true);
  } finally { demoMailGateway.updateMessage = originalUpdate; demoMailGateway.requestRecovery = originalRecovery; }
});

test('a failed recovery request keeps cached edits and returns failure without claiming a queued check', async () => {
  const originalRecovery = demoMailGateway.requestRecovery;
  const message = { ...structuredClone(demoWorkspace.messages[0]), stateRevision: 2 };
  try {
    useWorkspaceStore.setState({ ...structuredClone(demoWorkspace), messages: [message], status: 'ready', savingIds: [], error: null });
    demoMailGateway.requestRecovery = async () => { throw new Error('恢复请求未保存'); };
    assert.equal(await useWorkspaceStore.getState().requestRecovery(message.id, 'confirm'), false); await settle();
    assert.deepEqual(useWorkspaceStore.getState().messages[0], message);
    assert.match(useWorkspaceStore.getState().error ?? '', /恢复请求未保存/);
  } finally { demoMailGateway.requestRecovery = originalRecovery; }
});

test('continuing a recovery page preserves the pending decision and rejects late lower-version progress', async () => {
  const originalRecovery = demoMailGateway.requestRecovery; const originalLoad = demoMailGateway.loadWorkspace;
  const message: MailMessage = { ...structuredClone(demoWorkspace.messages[0]), stateRevision: 10, recovery: { state: 'scanning', detail: '继续核对', location: null, evidence: 'raw-sha256', progress: { checked: 8, total: 101, matches: 1 } } };
  let release!: (message: MailMessage) => void;
  try {
    useWorkspaceStore.setState({ ...structuredClone(demoWorkspace), messages: [message], status: 'ready', savingIds: [], error: null });
    demoMailGateway.requestRecovery = async (_account, _id, revision, action) => { assert.equal(revision, 10); assert.equal(action, 'continue'); return new Promise(resolve => { release = resolve; }); };
    const request = useWorkspaceStore.getState().requestRecovery(message.id, 'continue'); await tick();
    const newer: MailMessage = { ...message, stateRevision: 13, recovery: { state: 'unresolved', detail: '候选改变', location: null, evidence: null, progress: null } };
    demoMailGateway.loadWorkspace = async () => ({ ...structuredClone(demoWorkspace), messages: [newer] });
    await useWorkspaceStore.getState().reload(); release({ ...message, stateRevision: 11, recovery: { ...message.recovery!, state: 'queued' } });
    assert.equal(await request, true); await settle(); assert.deepEqual(useWorkspaceStore.getState().messages[0], newer);
  } finally { demoMailGateway.requestRecovery = originalRecovery; demoMailGateway.loadWorkspace = originalLoad; }
});

test('fingerprint confirmation forwards its own operation without changing cached edits', async () => {
  const original = demoMailGateway.requestRecovery;
  const message: MailMessage = { ...structuredClone(demoWorkspace.messages[0]), stateRevision: 4, recovery: { state: 'located', operation: 'fingerprint', detail: '确认补齐', location: null, evidence: 'raw-sha256', fingerprint: { eligible: true, reason: '原定位已核对' } } };
  try {
    useWorkspaceStore.setState({ ...structuredClone(demoWorkspace), messages: [message], status: 'ready', savingIds: [], error: null });
    demoMailGateway.requestRecovery = async (_account, _id, revision, action) => {
      assert.equal(action, 'confirm-fingerprint'); assert.equal(revision, 4);
      return { ...message, stateRevision: 5, recovery: { ...message.recovery!, state: 'queued' } };
    };
    assert.equal(await useWorkspaceStore.getState().requestRecovery(message.id, 'confirm-fingerprint'), true); await settle();
    assert.equal(useWorkspaceStore.getState().messages[0].starred, message.starred);
    assert.equal(useWorkspaceStore.getState().messages[0].recovery?.operation, 'fingerprint');
  } finally { demoMailGateway.requestRecovery = original; }
});
