import { useEffect, useState } from 'react';
import { PenLine, Plus, Star, Trash2 } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { InlineConfirmation } from '../../components/ui/InlineConfirmation';
import { RichEditor } from '../compose/RichEditor';
import { useSignatureStore } from '../../stores/signatureStore';
import { errorMessage } from '../../domain/errors';
import { MAX_SIGNATURE_NAME_CHARS, type AccountSignature, type SignatureEntry } from '../../domain/signature';
import { useI18n } from '../../i18n/react';
import { SettingsPageHeading } from './SettingsPageHeading';
import { useSettingsNavigation } from './settingsNavigation';

interface EditState { id: string | null; name: string; form: AccountSignature; makeDefault: boolean }

/**
 * 每账号多签名管理（v23）：签名列表（名称 + 默认星标 + 编辑 / 删除）+ 新建；
 * 编辑态复用富文本编辑器（含插入图片）。默认签名供撰写注入（Outlook 语义不变）。
 */
export function SignatureOptions({ accountId }: { accountId: string }) {
  const t = useI18n();
  const [entries, setEntries] = useState<SignatureEntry[] | null>(null);
  const [editing, setEditing] = useState<EditState | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState('');
  const navigate = useSettingsNavigation(dirty && editing !== null, busy || picking);

  async function refresh() {
    setError('');
    try { setEntries(await useSignatureStore.getState().list(accountId)); }
    catch (listError) { setError(errorMessage(listError)); }
  }
  useEffect(() => {
    let active = true;
    void useSignatureStore.getState().list(accountId).then(value => {
      if (active) setEntries(value);
    }).catch(error => { if (active) setError(errorMessage(error)); });
    return () => { active = false; };
  }, [accountId]);

  function startCreate() {
    setDirty(false);
    setEditing({ id: null, name: '', form: { html: '', useNew: true, useReply: true }, makeDefault: false });
  }
  function startEdit(entry: SignatureEntry) {
    setDirty(false);
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
    setDeletingId(null);
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

  const nameInvalid = !editing || !editing.name.trim() || [...editing.name.trim()].length > MAX_SIGNATURE_NAME_CHARS;
  return <>
    <SettingsPageHeading level="h4" title={editing ? editing.id ? t('settings.editSignature') : t('signature.new') : t('settings.signatureHeading')} description={editing ? undefined : t('signature.managerDescription')} />
      <div className="signature-options">
        {editing === null && <>
          {!entries && !error && <p role="status">{t('syncOptions.loading')}</p>}
          {!entries && error && <Button onClick={() => void refresh()}>{t('settings.retry')}</Button>}
          {entries && entries.length === 0 && <p className="muted">{t('signature.empty')}</p>}
          {entries && entries.length > 0 && <ul className="signature-list">
            {entries.map(entry => <li key={entry.id}>
              {deletingId === entry.id ? <InlineConfirmation message={t('signature.deleteConfirm', { name: entry.name })} confirmLabel={t('signature.deleteButton')} onConfirm={() => void remove(entry)} onCancel={() => setDeletingId(null)} /> : <><span title={entry.name}>{entry.name}</span>
              {entry.isDefault && <span className="label signature-default" title={t('signature.defaultBadge')}><Star size={12} />{t('signature.defaultBadge')}</span>}
              <Button variant="ghost" iconOnly aria-label={t('signature.editAria', { name: entry.name })} disabled={busy || deletingId !== null} onClick={() => startEdit(entry)}><PenLine size={14} aria-hidden /></Button>
              <Button variant="ghost" iconOnly className="danger-button" aria-label={t('signature.deleteAria', { name: entry.name })} disabled={busy || deletingId !== null} onClick={() => setDeletingId(entry.id)}><Trash2 size={14} aria-hidden /></Button></>}
            </li>)}
          </ul>}
          {entries && entries.length < 10 && <Button variant="secondary" onClick={startCreate} disabled={busy || deletingId !== null}><Plus size={15} aria-hidden />{t('signature.new')}</Button>}
        </>}
        {editing !== null && <div className="signature-edit" onChangeCapture={() => setDirty(true)}>
          <label htmlFor="signature-name">{t('signature.nameAria')}</label>
          <input id="signature-name" name="signatureName" autoComplete="off" className="signature-name" value={editing.name} placeholder={t('signature.namePlaceholder')} maxLength={200} disabled={busy} autoFocus onChange={event => setEditing({ ...editing, name: event.target.value })} />
          <RichEditor key={editing.id ?? 'new'} initialHtml={editing.form.html} initialText="" onChange={html => { setDirty(true); setEditing(current => current && { ...current, form: { ...current.form, html } }); }} disabled={busy} onPickImage={pickImage} />
          <label><input type="checkbox" checked={editing.form.useNew} disabled={busy} onChange={event => setEditing({ ...editing, form: { ...editing.form, useNew: event.target.checked } })} />{t('signature.useNew')}</label>
          <label><input type="checkbox" checked={editing.form.useReply} disabled={busy} onChange={event => setEditing({ ...editing, form: { ...editing.form, useReply: event.target.checked } })} />{t('signature.useReply')}</label>
          <label><input type="checkbox" checked={editing.makeDefault} disabled={busy} onChange={event => setEditing({ ...editing, makeDefault: event.target.checked })} />{t('signature.makeDefault')}</label>
        </div>}
        {error && <p className="inline-error" role="alert">{error}</p>}
        {editing !== null && <div className="settings-form-actions"><Button disabled={busy || picking} onClick={() => navigate?.(() => setEditing(null))}>{t('editor.cancel')}</Button><Button variant="primary" disabled={busy || picking || nameInvalid} onClick={() => void save()}>{busy ? t('wizard.saving') : t('syncOptions.save')}</Button></div>}
      </div>
  </>;
}
