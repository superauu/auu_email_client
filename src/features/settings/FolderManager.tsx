import { useState } from 'react';
import { PenLine, Trash2 } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { InlineConfirmation } from '../../components/ui/InlineConfirmation';
import { desktopFolders } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useShallow } from 'zustand/react/shallow';
import { useI18n } from '../../i18n/react';
import type { MailFolder } from '../../domain/mail';
import { SettingsPageHeading } from './SettingsPageHeading';
import { useSettingsNavigation } from './settingsNavigation';

/** 服务器目录管理（仅 IMAP 自定义目录）：新建 / 重命名 / 删除，服务器确认后更新本机缓存。 */
export function FolderManager({ accountId }: { accountId: string }) {
  const t = useI18n();
  const [folders, setFolders] = useState<MailFolder[] | null>(null);
  const [error, setError] = useState('');
  const [newName, setNewName] = useState('');
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 目录列表从工作区 store 取（侧栏同源）；CRUD 后用 Rust 返回的最新列表覆盖并 reload。
  const storeFolders = useWorkspaceStore(useShallow(s => s.folders));
  const list = folders ?? storeFolders?.filter(folder => folder.accountId === accountId) ?? [];
  const custom = list.filter(folder => folder.kind === 'other');
  const renaming = custom.find(folder => folder.id === renamingId);
  useSettingsNavigation(Boolean(newName.trim() || (renaming && renameValue !== renaming.name)), busy);

  function refresh(list: MailFolder[]) {
    setFolders(list);
    // 目录缓存已由 Rust 更新；刷新工作区让侧栏目录树同步。
    void useWorkspaceStore.getState().reload().catch(() => undefined);
  }

  async function create() {
    if (!newName.trim() || busy) return;
    setBusy(true); setError('');
    try {
      refresh(await desktopFolders.create(accountId, newName.trim()));
      setNewName('');
    } catch (error) { setError(errorMessage(error)); }
    setBusy(false);
  }
  async function rename(folderId: string) {
    if (!renameValue.trim() || busy) return;
    setBusy(true); setError('');
    try {
      refresh(await desktopFolders.rename(accountId, folderId, renameValue.trim()));
      setRenamingId(null); setRenameValue('');
    } catch (error) { setError(errorMessage(error)); }
    setBusy(false);
  }
  async function remove(folder: MailFolder) {
    if (busy) return;
    setDeletingId(null);
    setBusy(true); setError('');
    try {
      refresh(await desktopFolders.remove(accountId, folder.id));
    } catch (error) { setError(errorMessage(error)); }
    setBusy(false);
  }

  return <>
    <SettingsPageHeading level="h4" title={t('folders.button')} description={t('folders.description')} />
      <div className="folder-manager">
        <form className="folder-create" onSubmit={event => { event.preventDefault(); void create(); }}>
            <label className="sr-only" htmlFor={`new-folder-${accountId}`}>{t('folders.createLabel')}</label>
            <input id={`new-folder-${accountId}`} name="folderName" autoComplete="off" value={newName} placeholder={t('folders.createPlaceholder')} aria-label={t('folders.createAria')} maxLength={200} disabled={busy} onChange={event => setNewName(event.target.value)} />
          <Button type="submit" disabled={busy || !newName.trim()}>{t('folders.createButton')}</Button>
        </form>
        {custom.length === 0 ? <p className="muted">{t('folders.empty')}</p> : <ul className="folder-list-manage">
            {custom.map(folder => <li key={folder.id}>
              {deletingId === folder.id ? <InlineConfirmation message={t('folders.deleteConfirm', { name: folder.name })} confirmLabel={t('folders.deleteButton')} onConfirm={() => void remove(folder)} onCancel={() => setDeletingId(null)} /> : renamingId === folder.id ? <form className="folder-rename" onSubmit={event => { event.preventDefault(); void rename(folder.id); }}>
                <input name="folderRename" autoComplete="off" value={renameValue} aria-label={t('folders.renameAria', { name: folder.name })} maxLength={200} disabled={busy} onChange={event => setRenameValue(event.target.value)} autoFocus />
                <Button type="submit" disabled={busy || !renameValue.trim() || renameValue.trim() === folder.name}>{t('folders.renameSave')}</Button>
                <Button variant="ghost" disabled={busy} onClick={() => { setRenamingId(null); setRenameValue(''); }}>{t('folders.cancel')}</Button>
                <p className="muted">{t('folders.renameNote')}</p>
              </form> : <>
                <span title={folder.name}>{folder.name}</span>
                <Button variant="ghost" iconOnly aria-label={t('folders.renameAria', { name: folder.name })} disabled={busy || Boolean(renamingId || deletingId)} onClick={() => { setRenamingId(folder.id); setRenameValue(folder.name); }}><PenLine size={14} aria-hidden /></Button>
                <Button variant="ghost" iconOnly className="danger-button" aria-label={t('folders.deleteAria', { name: folder.name })} disabled={busy || Boolean(renamingId || deletingId)} onClick={() => setDeletingId(folder.id)}><Trash2 size={14} aria-hidden /></Button>
              </>}
            </li>)}
          </ul>}
        {error && <p className="inline-error" role="alert">{error}</p>}
        {busy && <p className="muted" role="status">{t('settings.operationInProgress')}</p>}
      </div>
  </>;
}
