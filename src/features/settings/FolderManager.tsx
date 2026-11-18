import { useState } from 'react';
import { Folder, PenLine, Trash2 } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import { desktopFolders, isDesktop } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useShallow } from 'zustand/react/shallow';
import { useI18n } from '../../i18n/react';
import type { MailFolder, ProviderKind } from '../../domain/mail';

/** 服务器目录管理（仅 IMAP 自定义目录）：新建 / 重命名 / 删除，服务器确认后更新本机缓存。 */
export function FolderManager({ accountId, name, provider }: { accountId: string; name: string; provider: ProviderKind }) {
  const t = useI18n();
  const [open, setOpen] = useState(false);
  const [folders, setFolders] = useState<MailFolder[] | null>(null);
  const [error, setError] = useState('');
  const [newName, setNewName] = useState('');
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [busy, setBusy] = useState(false);
  const supported = isDesktop() && provider === 'imap';
  // 目录列表从工作区 store 取（侧栏同源）；CRUD 后用 Rust 返回的最新列表覆盖并 reload。
  const storeFolders = useWorkspaceStore(useShallow(s => s.folders));
  const list = folders ?? storeFolders?.filter(folder => folder.accountId === accountId) ?? [];
  const custom = list.filter(folder => folder.kind === 'other');

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
    if (!window.confirm(t('folders.deleteConfirm', { name: folder.name }))) return;
    setBusy(true); setError('');
    try {
      refresh(await desktopFolders.remove(accountId, folder.id));
    } catch (error) { setError(errorMessage(error)); }
    setBusy(false);
  }

  if (!supported) return null;
  return <><Button variant="ghost" aria-label={t('folders.buttonAria', { name })} onClick={() => { setFolders(null); setError(''); setOpen(true); }}><Folder size={14} />{t('folders.button')}</Button>
    <Dialog open={open} onOpenChange={value => { if (!busy) setOpen(value); }} title={t('folders.title', { name })} description={t('folders.description')}>
      <div className="folder-manager">
        <div className="folder-create">
            <label className="sr-only" htmlFor={`new-folder-${accountId}`}>{t('folders.createLabel')}</label>
            <input id={`new-folder-${accountId}`} value={newName} placeholder={t('folders.createPlaceholder')} aria-label={t('folders.createAria')} maxLength={200} disabled={busy} onChange={event => setNewName(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') void create(); }} />
          <Button disabled={busy || !newName.trim()} onClick={() => void create()}>{t('folders.createButton')}</Button>
        </div>
        {custom.length === 0 ? <p className="muted">{t('folders.empty')}</p> : <ul className="folder-list-manage">
            {custom.map(folder => <li key={folder.id}>
              {renamingId === folder.id ? <>
                <input value={renameValue} aria-label={t('folders.renameAria', { name: folder.name })} maxLength={200} disabled={busy} onChange={event => setRenameValue(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') void rename(folder.id); }} autoFocus />
                <Button disabled={busy || !renameValue.trim()} onClick={() => void rename(folder.id)}>{t('folders.renameSave')}</Button>
                <Button variant="ghost" disabled={busy} onClick={() => { setRenamingId(null); setRenameValue(''); }}>{t('folders.cancel')}</Button>
              </> : <>
                <span title={folder.name}>{folder.name}</span>
                <Button variant="ghost" iconOnly aria-label={t('folders.renameAria', { name: folder.name })} disabled={busy} onClick={() => { if (window.confirm(t('folders.renameConfirm'))) { setRenamingId(folder.id); setRenameValue(folder.name); } }}><PenLine size={14} /></Button>
                <Button variant="ghost" iconOnly className="folder-delete" aria-label={t('folders.deleteAria', { name: folder.name })} disabled={busy} onClick={() => void remove(folder)}><Trash2 size={14} /></Button>
              </>}
            </li>)}
          </ul>}
        {error && <p className="inline-error" role="alert">{error}</p>}
      </div>
    </Dialog></>;
}
