import test from 'node:test';
import assert from 'node:assert/strict';
import { useWorkspaceStore } from '../src/stores/workspaceStore';
import { demoMailGateway } from '../src/services/demo/DemoMailGateway';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { WorkspaceSnapshot } from '../src/domain/mail';

test('background reload keeps the workspace visible and retains a concurrently saved draft', async () => {
  const originalLoad = demoMailGateway.loadWorkspace; const originalSave = demoMailGateway.saveDraft;
  let resolve!: (value: WorkspaceSnapshot) => void;
  try {
    useWorkspaceStore.setState({ ...structuredClone(demoWorkspace), status: 'ready' });
    demoMailGateway.loadWorkspace = () => new Promise(done => { resolve = done; });
    demoMailGateway.saveDraft = async draft => ({ ...demoWorkspace.messages[0], id: draft.id, accountId: draft.accountId, mailbox: 'drafts', draft: { ...draft, revision: 1 } });
    const reload = useWorkspaceStore.getState().reload();
    assert.equal(useWorkspaceStore.getState().status, 'ready');
    assert.equal(useWorkspaceStore.getState().reload(), reload);
    await useWorkspaceStore.getState().saveDraft({ id: 'draft-concurrent', accountId: 'personal', to: '', subject: 'Keep', bodyText: 'Latest text', updatedAt: new Date().toISOString() });
    resolve(structuredClone(demoWorkspace)); await reload;
    assert.equal(useWorkspaceStore.getState().messages.find(message => message.id === 'draft-concurrent')?.draft?.bodyText, 'Latest text');
    demoMailGateway.loadWorkspace = async () => { throw new Error('Read failure'); };
    await assert.rejects(useWorkspaceStore.getState().reload());
    assert.equal(useWorkspaceStore.getState().status, 'ready');
    assert.ok(useWorkspaceStore.getState().messages.some(message => message.id === 'draft-concurrent'));
  } finally { demoMailGateway.loadWorkspace = originalLoad; demoMailGateway.saveDraft = originalSave; }
});
