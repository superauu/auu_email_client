import type { WorkspaceSnapshot } from './mail';
import { USER_LABEL_PREFIX } from './labels';

/** Legacy demo rows remain in storage but never enter the desktop UI snapshot. */
export function realWorkspace(snapshot: WorkspaceSnapshot): WorkspaceSnapshot {
  const accounts = snapshot.accounts.filter(account => account.status !== 'demo');
  const owners = new Set(accounts.map(account => account.id));
  const messages = snapshot.messages.filter(message => owners.has(message.accountId));
  const usedLabels = new Set(messages.flatMap(message => message.labelIds));
  return {
    accounts,
    messages,
    folders: (snapshot.folders ?? []).filter(folder => owners.has(folder.accountId)),
    // Older demo labels were global. Only offer global labels used by real mail;
    // user-created tags (lbl: prefix) always stay visible.
    labels: snapshot.labels.filter(label =>
      label.accountId ? owners.has(label.accountId) : usedLabels.has(label.id) || label.id.startsWith(USER_LABEL_PREFIX),
    ),
    events: snapshot.events.filter(event => owners.has(event.accountId)),
    syncStates: Object.fromEntries(Object.entries(snapshot.syncStates ?? {}).filter(([id]) => owners.has(id))),
  };
}
