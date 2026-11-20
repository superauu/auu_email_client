import { useEffect, useState } from 'react';
import { PenLine, Plus, Star, Trash2 } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import { RichEditor } from '../compose/RichEditor';
import { useSignatureStore } from '../../stores/signatureStore';
import { errorMessage } from '../../domain/errors';
import { MAX_SIGNATURE_NAME_CHARS, type AccountSignature, type SignatureEntry } from '../../domain/signature';
import { isDesktop } from '../../platform/desktop';
import { useI18n } from '../../i18n/react';

interface EditState { id: string | null; name: string; form: AccountSignature; makeDefault: boolean }

/**
 * 每账号多签名管理（v23）：签名列表（名称 + 默认星标 + 编辑 / 删除）+ 新建；
 * 编辑态复用富文本编辑器（含插入图片）。默认签名供撰写注入（Outlook 语义不变）。
 */
export function SignatureOptions({ accountId, name }: { accountId: string; name: string }) {
  const t = useI18n();
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<SignatureEntry[] | null>(null);
  const [editing, setEditing] = useState<EditState | null>(null);
  const [busy, setBusy] = useState(false);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState('');

  async function refresh() {
    try { setEntries(await useSignatureStore.getState().list(accountId)); }
    catch (listError) { setError(errorMessage(listError)); }
  }
  useEffect(() => {
    if (!open) return;
    setEntries(null);
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, accountId]);

  function startCreate() {
    setEditing({ id: null, name: '', form: { html: '', useNew: true, useReply: true }, makeDefault: false });
  }
  function startEdit(entry: SignatureEntry) {
    setEditing({ id: entry.id, name: entry.name, form: { html: entry.html, useNew: entry.useNew, useReply: entry.useReply }, makeDefault: false });
  }
  async function save() {
    if (!editing || busy) return;
    setBusy(true); setError('');
    try {
      await useSignatureStore.getState().saveEntry(accountId, editing);
      setEditing(null);
      await refresh();
    } catch (saveError) { setError(errorMessage(saveError)); }
    setBusy(false);
  }
  async function remove(entry: SignatureEntry) {
    if (busy) return;
    if (!window.confirm(t('signature.deleteConfirm', { name: entry.name }))) return;
    setBusy(true); setError('');
    try {
      await useSignatureStore.getState().removeEntry(accountId, entry.id);
      await refresh();
    } catch (removeError) { setError(errorMessage(removeError)); }
    setBusy(false);
  }
  /** 图片选取交由 Rust 对话框；返回 data URI 由 RichEditor 插入光标处。 */
  async function pickImage() {
    setPicking(true);
    try { return await useSignatureStore.getState().pickImage(); }
    catch (imageError) { setError(errorMessage(imageError)); return null; }
    finally { setPicking(false); }
  }

  if (!isDesktop()) return null;
  const nameInvalid = !editing || !editing.name.trim() || [...editing.name.trim()].length > MAX_SIGNATURE_NAME_CHARS;
  return <><Button variant="ghost" aria-label={t('signature.buttonAria', { name })} onClick={() => { setEditing(null); setError(''); setOpen(true); }}>{t('signature.button')}</Button>
    <Dialog open={open} onOpenChange={value => { if (!busy) setOpen(value); }} title={t('signature.title', { name })} description={t('signature.managerDescription')}>
      <div className="signature-options">
        {editing === null && <>
          {!entries && !error && <p role="status">{t('syncOptions.loading')}</p>}
          {entries && entries.length === 0 && <p className="muted">{t('signature.empty')}</p>}
          {entries && entries.length > 0 && <ul className="signature-list">
            {entries.map(entry => <li key={entry.id}>
              <span title={entry.name}>{entry.name}</span>
              {entry.isDefault && <span className="label signature-default" title={t('signature.defaultBadge')}><Star size={12} />{t('signature.defaultBadge')}</span>}
              <Button variant="ghost" iconOnly aria-label={t('signature.editAria', { name: entry.name })} disabled={busy} onClick={() => startEdit(entry)}><PenLine size={14} /></Button>
              <Button variant="ghost" iconOnly className="folder-delete" aria-label={t('signature.deleteAria', { name: entry.name })} disabled={busy} onClick={() => void remove(entry)}><Trash2 size={14} /></Button>
            </li>)}
          </ul>}
          {entries && entries.length < 10 && <Button variant="secondary" onClick={startCreate} disabled={busy}><Plus size={15} />{t('signature.new')}</Button>}
        </>}
        {editing !== null && <>
          <input className="signature-name" value={editing.name} placeholder={t('signature.namePlaceholder')} aria-label={t('signature.nameAria')} maxLength={200} disabled={busy} onChange={event => setEditing({ ...editing, name: event.target.value })} />
          <RichEditor key={editing.id ?? 'new'} initialHtml={editing.form.html} initialText="" onChange={html => setEditing({ ...editing, form: { ...editing.form, html } })} disabled={busy} onPickImage={pickImage} />
          <label><input type="checkbox" checked={editing.form.useNew} disabled={busy} onChange={event => setEditing({ ...editing, form: { ...editing.form, useNew: event.target.checked } })} />{t('signature.useNew')}</label>
          <label><input type="checkbox" checked={editing.form.useReply} disabled={busy} onChange={event => setEditing({ ...editing, form: { ...editing.form, useReply: event.target.checked } })} />{t('signature.useReply')}</label>
          <label><input type="checkbox" checked={editing.makeDefault} disabled={busy} onChange={event => setEditing({ ...editing, makeDefault: event.target.checked })} />{t('signature.makeDefault')}</label>
        </>}
        {error && <p className="inline-error" role="alert">{error}</p>}
        <div className="compose-footer">
          {editing !== null
            ? <><Button disabled={busy} onClick={() => setEditing(null)}>{t('editor.cancel')}</Button><Button variant="primary" disabled={busy || picking || nameInvalid} onClick={() => void save()}>{busy ? t('wizard.saving') : t('syncOptions.save')}</Button></>
            : <Button disabled={busy} onClick={() => setOpen(false)}>{t('editor.cancel')}</Button>}
        </div>
      </div>
    </Dialog></>;
}
