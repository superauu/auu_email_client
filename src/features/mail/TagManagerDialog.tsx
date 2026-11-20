import { useState } from 'react';
import { PenLine, Tags, Trash2 } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import { isDesktop } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { isUserLabel, isLabelColor, labelNameError, LABEL_COLORS, type LabelColor } from '../../domain/labels';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useI18n } from '../../i18n/react';

const COLOR_LABEL: Record<LabelColor, 'tags.colorTeal' | 'tags.colorBlue' | 'tags.colorAmber'> = { teal: 'tags.colorTeal', blue: 'tags.colorBlue', amber: 'tags.colorAmber' };

/** 用户自建标签管理（桌面端）：新建 / 重命名 / 改色 / 删除；纯本机注解，无服务器语义。 */
export function TagManagerDialog() {
  const t = useI18n();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState('');
  const [newName, setNewName] = useState('');
  const [newColor, setNewColor] = useState<LabelColor>('teal');
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [renameColor, setRenameColor] = useState<LabelColor>('teal');
  const [busy, setBusy] = useState(false);
  const supported = isDesktop();
  const labels = useWorkspaceStore(useShallow(s => s.labels));
  const { createLabel, renameLabel, deleteLabel } = useWorkspaceStore(useShallow(s => ({ createLabel: s.createLabel, renameLabel: s.renameLabel, deleteLabel: s.deleteLabel })));
  const manageable = labels.filter(isUserLabel);

  async function create() {
    const invalid = labelNameError(newName);
    if (invalid || busy) { setError(invalid); return; }
    setBusy(true); setError('');
    try {
      await createLabel(newName.trim(), newColor);
      setNewName(''); setNewColor('teal');
    } catch (caught) { setError(errorMessage(caught)); }
    setBusy(false);
  }
  function startRename(id: string, name: string, color: string) {
    setRenamingId(id); setRenameValue(name); setRenameColor(isLabelColor(color) ? color : 'teal');
  }
  async function saveRename() {
    const invalid = labelNameError(renameValue);
    if (invalid || busy || !renamingId) { if (invalid) setError(invalid); return; }
    setBusy(true); setError('');
    try {
      await renameLabel(renamingId, renameValue.trim(), renameColor);
      setRenamingId(null); setRenameValue('');
    } catch (caught) { setError(errorMessage(caught)); }
    setBusy(false);
  }
  async function remove(id: string, name: string) {
    if (busy) return;
    if (!window.confirm(t('tags.deleteConfirm', { name }))) return;
    setBusy(true); setError('');
    try { await deleteLabel(id); } catch (caught) { setError(errorMessage(caught)); }
    setBusy(false);
  }

  if (!supported) return null;
  return <><Button variant="ghost" iconOnly aria-label={t('tags.manageAria')} title={t('tags.manageAria')} onClick={() => { setError(''); setOpen(true); }}><Tags size={14} /></Button>
    <Dialog open={open} onOpenChange={value => { if (!busy) setOpen(value); }} title={t('tags.title')} description={t('tags.description')}>
      <div className="folder-manager">
        <div className="folder-create">
          <label className="sr-only" htmlFor="new-tag-name">{t('tags.createAria')}</label>
          <input id="new-tag-name" value={newName} placeholder={t('tags.createPlaceholder')} aria-label={t('tags.createAria')} maxLength={200} disabled={busy} onChange={event => setNewName(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') void create(); }} />
          <div className="tag-color-options" role="group" aria-label={t('tags.colorGroupAria')}>
            {LABEL_COLORS.map(color => <button key={color} type="button" className={`tag-color-option${newColor === color ? ' is-active' : ''}`} aria-label={t(COLOR_LABEL[color])} aria-pressed={newColor === color} disabled={busy} onClick={() => setNewColor(color)}><span className={`label-dot tone-${color}`} /></button>)}
          </div>
          <Button disabled={busy || !!labelNameError(newName)} onClick={() => void create()}>{t('tags.createButton')}</Button>
        </div>
        {manageable.length === 0 ? <p className="muted">{t('tags.empty')}</p> : <ul className="folder-list-manage">
          {manageable.map(label => <li key={label.id}>
            {renamingId === label.id ? <>
              <input value={renameValue} aria-label={t('tags.renameAria', { name: label.name })} maxLength={200} disabled={busy} onChange={event => setRenameValue(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') void saveRename(); }} autoFocus />
              <div className="tag-color-options" role="group" aria-label={t('tags.colorGroupAria')}>
                {LABEL_COLORS.map(color => <button key={color} type="button" className={`tag-color-option${renameColor === color ? ' is-active' : ''}`} aria-label={t(COLOR_LABEL[color])} aria-pressed={renameColor === color} disabled={busy} onClick={() => setRenameColor(color)}><span className={`label-dot tone-${color}`} /></button>)}
              </div>
              <Button disabled={busy || !!labelNameError(renameValue)} onClick={() => void saveRename()}>{t('tags.renameSave')}</Button>
              <Button variant="ghost" disabled={busy} onClick={() => { setRenamingId(null); setRenameValue(''); }}>{t('folders.cancel')}</Button>
            </> : <>
              <span className={`label-dot tone-${label.color}`} aria-hidden />
              <span title={label.name}>{label.name}</span>
              <Button variant="ghost" iconOnly aria-label={t('tags.renameAria', { name: label.name })} disabled={busy} onClick={() => startRename(label.id, label.name, label.color)}><PenLine size={14} /></Button>
              <Button variant="ghost" iconOnly className="folder-delete" aria-label={t('tags.deleteAria', { name: label.name })} disabled={busy} onClick={() => void remove(label.id, label.name)}><Trash2 size={14} /></Button>
            </>}
          </li>)}
        </ul>}
        {error && <p className="inline-error" role="alert">{error}</p>}
      </div>
    </Dialog></>;
}
