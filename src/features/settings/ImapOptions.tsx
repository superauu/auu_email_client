import { useEffect, useState } from 'react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import { useImapStore } from '../../stores/imapStore';
import { errorMessage } from '../../domain/errors';
import type { ImapSettings } from '../../domain/imap';
import type { ProviderKind } from '../../domain/mail';
import { useI18n } from '../../i18n/react';
export function ImapOptions({ accountId, name, provider }: { accountId: string; name: string; provider: ProviderKind }) {
  const t = useI18n();
  const pop3 = provider === 'pop3';
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState<ImapSettings | null>(null);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (!open) return;
    let active = true;
    void useImapStore.getState().load(accountId).then(value => { if (active) setForm(value); }).catch(error => { if (active) setError(errorMessage(error)); });
    return () => { active = false; };
  }, [open, accountId]);
  async function save() {
    if (!form) return;
    setSaving(true); setError('');
    try { await useImapStore.getState().save(accountId, pop3 ? { ...form, syncAllFolders: false, historyEnabled: false } : form); setOpen(false); }
    catch (error) { setError(errorMessage(error)); }
    setSaving(false);
  }
  return <><Button variant="ghost" aria-label={t('syncOptions.buttonAria', { name })} onClick={() => { setForm(null); setError(''); setOpen(true); }}>{t('syncOptions.button')}</Button>
    <Dialog open={open} onOpenChange={value => { if (!saving) setOpen(value); }} title={t('syncOptions.title', { name, protocol: pop3 ? 'POP3' : 'IMAP' })} description={t('syncOptions.description')}>
      <div className="imap-options">
        {!form && !error && <p role="status">{t('syncOptions.loading')}</p>}
        {form && <>
          {!pop3 && <><label><input type="checkbox" checked={form.syncAllFolders} disabled={saving} onChange={event => setForm({ ...form, syncAllFolders: event.target.checked })} />{t('syncOptions.allFolders')}</label>
          <label><input type="checkbox" checked={form.historyEnabled} disabled={saving} onChange={event => setForm({ ...form, historyEnabled: event.target.checked })} />{t('syncOptions.history')}</label></>}
          <label>{t('syncOptions.autoLabel')}<select aria-label={t('syncOptions.autoLabel')} value={form.autoSyncSeconds} disabled={saving} onChange={event => setForm({ ...form, autoSyncSeconds: Number(event.target.value) })}><option value={0}>{t('syncOptions.autoOff')}</option><option value={60}>{t('syncOptions.autoMinute')}</option><option value={300}>{t('syncOptions.auto5Minutes')}</option><option value={900}>{t('syncOptions.auto15Minutes')}</option></select></label>
          <p className="muted">{t('syncOptions.noteCommon')}{pop3 ? t('syncOptions.notePop3') : t('syncOptions.noteImapHistory')}</p>
        </>}
        {error && <p className="inline-error" role="alert">{error}</p>}
        <div className="compose-footer"><Button disabled={saving} onClick={() => setOpen(false)}>{t('editor.cancel')}</Button><Button variant="primary" disabled={!form || saving} onClick={() => void save()}>{saving ? t('wizard.saving') : t('syncOptions.save')}</Button></div>
      </div>
    </Dialog></>;
}
