import { useEffect, useState } from 'react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import { RichEditor } from '../compose/RichEditor';
import { useSignatureStore } from '../../stores/signatureStore';
import { errorMessage } from '../../domain/errors';
import type { AccountSignature } from '../../domain/signature';
import { isDesktop } from '../../platform/desktop';
import { useI18n } from '../../i18n/react';

/** 每账号邮件签名（真实账号）：富文本编辑（含插入图片）+ 新邮件 / 回复转发两个追加开关。 */
export function SignatureOptions({ accountId, name }: { accountId: string; name: string }) {
  const t = useI18n();
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState<AccountSignature | null>(null);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [picking, setPicking] = useState(false);
  useEffect(() => {
    if (!open) return;
    let active = true;
    void useSignatureStore.getState().load(accountId).then(value => { if (active) setForm(value); }).catch(error => { if (active) setError(errorMessage(error)); });
    return () => { active = false; };
  }, [open, accountId]);
  async function save() {
    if (!form) return;
    setSaving(true); setError('');
    try { await useSignatureStore.getState().save(accountId, form); setOpen(false); }
    catch (error) { setError(errorMessage(error)); }
    setSaving(false);
  }
  /** 图片选取交由 Rust 对话框；返回 data URI 由 RichEditor 插入光标处。 */
  async function pickImage() {
    setPicking(true);
    try { return await useSignatureStore.getState().pickImage(); }
    catch (error) { setError(errorMessage(error)); return null; }
    finally { setPicking(false); }
  }
  if (!isDesktop()) return null;
  return <><Button variant="ghost" aria-label={t('signature.buttonAria', { name })} onClick={() => { setForm(null); setError(''); setOpen(true); }}>{t('signature.button')}</Button>
    <Dialog open={open} onOpenChange={value => { if (!saving) setOpen(value); }} title={t('signature.title', { name })} description={t('signature.description')}>
      <div className="signature-options">
        {!form && !error && <p role="status">{t('syncOptions.loading')}</p>}
        {form && <>
          <RichEditor key={accountId} initialHtml={form.html} initialText="" onChange={html => setForm({ ...form, html })} disabled={saving} onPickImage={pickImage} />
          <label><input type="checkbox" checked={form.useNew} disabled={saving} onChange={event => setForm({ ...form, useNew: event.target.checked })} />{t('signature.useNew')}</label>
          <label><input type="checkbox" checked={form.useReply} disabled={saving} onChange={event => setForm({ ...form, useReply: event.target.checked })} />{t('signature.useReply')}</label>
        </>}
        {error && <p className="inline-error" role="alert">{error}</p>}
        <div className="compose-footer"><Button disabled={saving} onClick={() => setOpen(false)}>{t('editor.cancel')}</Button><Button variant="primary" disabled={!form || saving || picking} onClick={() => void save()}>{saving ? t('wizard.saving') : t('syncOptions.save')}</Button></div>
      </div>
    </Dialog></>;
}
