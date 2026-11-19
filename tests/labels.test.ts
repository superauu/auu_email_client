import test from 'node:test';
import assert from 'node:assert/strict';
import { isLabelColor, isUserLabel, labelNameError, MAX_LABEL_NAME_CHARS, USER_LABEL_PREFIX } from '../src/domain/labels';
import { realWorkspace } from '../src/domain/workspace';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { WorkspaceSnapshot } from '../src/domain/mail';

test('label name validation trims, rejects empty and over-length names', () => {
  assert.equal(labelNameError('工作'), '');
  assert.equal(labelNameError('  工作  '), '');
  assert.notEqual(labelNameError('   '), '');
  assert.notEqual(labelNameError(''), '');
  assert.notEqual(labelNameError('标'.repeat(MAX_LABEL_NAME_CHARS + 1)), '');
  assert.equal(labelNameError('标'.repeat(MAX_LABEL_NAME_CHARS)), '');
});

test('label color and user-prefix helpers', () => {
  assert.equal(isLabelColor('teal'), true);
  assert.equal(isLabelColor('amber'), true);
  assert.equal(isLabelColor('purple'), false);
  assert.equal(isUserLabel({ id: `${USER_LABEL_PREFIX}abc`, name: 'x', color: 'teal', scope: 'local' }), true);
  assert.equal(isUserLabel({ id: 'design', name: 'x', color: 'teal', scope: 'local' }), false);
});

test('real workspace always keeps user-created labels but hides unused legacy demo labels', () => {
  const real = { ...demoWorkspace.accounts[0], id: 'added', status: 'offline' as const };
  const message = { ...demoWorkspace.messages[0], id: 'real-message', accountId: real.id, labelIds: [] };
  const snapshot: WorkspaceSnapshot = {
    ...structuredClone(demoWorkspace),
    accounts: [...demoWorkspace.accounts, real],
    messages: [...demoWorkspace.messages, message],
    labels: [
      ...demoWorkspace.labels,
      { id: `${USER_LABEL_PREFIX}mine`, name: '我的标签', color: 'teal', scope: 'local' },
    ],
    folders: [],
    events: [],
    syncStates: { added: '2026-11-20T00:00:00Z' },
  };
  const result = realWorkspace(snapshot);
  // 历史演示标签未被真实邮件使用 → 隐藏；用户自建标签始终可见。
  assert.deepEqual(result.labels.map(label => label.id), [`${USER_LABEL_PREFIX}mine`]);
});
