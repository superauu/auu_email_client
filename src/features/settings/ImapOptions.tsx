import { useEffect, useState } from 'react';
import { Button } from '../../components/ui/Button';
import { useImapStore } from '../../stores/imapStore';
import { errorMessage } from '../../domain/errors';
import type { ImapSettings } from '../../domain/imap';
import type { ProviderKind } from '../../domain/mail';
import { useI18n } from '../../i18n/react';
import { SettingsPageHeading } from './SettingsPageHeading';
import { useSettingsNavigation } from './settingsNavigation';

export function ImapOptions({ accountId, provider }: { accountId: string; provider: ProviderKind }) {
  const t = useI18n();
  const pop3 = provider === 'pop3';
  const [saved, setSaved] = useState<ImapSettings | null>(null);
  const [form, setForm] = useState<ImapSettings | null>(null);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const dirty = form !== null && saved !== null && (
    form.syncAllFolders !== saved.syncAllFolders || form.historyEnabled !== saved.historyEnabled
    || form.autoSyncSeconds !== saved.autoSyncSeconds
  );
  useSettingsNavigation(dirty, saving);

  useEffect(() => {
    let active = true;
    void useImapStore.getState().load(accountId).then(value => {
      if (active) { setSaved(value); setForm(value); }
    }).catch(error => { if (active) setError(errorMessage(error)); });
    return () => { active = false; };
  }, [accountId, loadAttempt]);

  async function save() {
    if (!form || !dirty || saving) return;
    setSaving(true);
    setError('');
    try {
      await useImapStore.getState().save(accountId, pop3 ? { ...form, syncAllFolders: false, historyEnabled: false } : form);
      setSaved(form);
      setConfirmed(true);
    } catch (error) { setError(errorMessage(error)); }
    finally { setSaving(false); }
  }

  return <>
    <SettingsPageHeading level="h4" title={t('settings.syncSectionTitle', { protocol: pop3 ? 'POP3' : 'IMAP' })} description={t('syncOptions.description')} />
    {!form && !error && <p role="status">{t('syncOptions.loading')}</p>}
    {form && <form className="settings-form" onSubmit={event => { event.preventDefault(); void save(); }} onChange={() => setConfirmed(false)}>
      <label className="settings-field">
        <span id="settings-auto-sync-label">{t('syncOptions.autoLabel')}</span>
        <select name="autoSyncSeconds" aria-labelledby="settings-auto-sync-label" value={form.autoSyncSeconds} disabled={saving} onChange={event => setForm({ ...form, autoSyncSeconds: Number(event.target.value) })}>
          <option value={0}>{t('syncOptions.autoOff')}</option>
          <option value={60}>{t('syncOptions.autoMinute')}</option>
          <option value={300}>{t('syncOptions.auto5Minutes')}</option>
          <option value={900}>{t('syncOptions.auto15Minutes')}</option>
        </select>
      </label>
      {!pop3 && <>
        <label className="settings-field"><span>{t('syncOptions.allFolders')}</span><input name="syncAllFolders" type="checkbox" checked={form.syncAllFolders} disabled={saving} onChange={event => setForm({ ...form, syncAllFolders: event.target.checked })} /></label>
        <label className="settings-field"><span>{t('syncOptions.history')}</span><input name="historyEnabled" type="checkbox" checked={form.historyEnabled} disabled={saving} onChange={event => setForm({ ...form, historyEnabled: event.target.checked })} /></label>
      </>}
      <p className="muted">{t('syncOptions.noteCommon')}{pop3 ? t('syncOptions.notePop3') : t('syncOptions.noteImapHistory')}</p>
      <div className="settings-form-actions">
        <Button variant="primary" type="submit" disabled={!dirty || saving}>{saving ? t('wizard.saving') : t('syncOptions.save')}</Button>
        {confirmed && <p role="status" className="settings-saved">{t('settings.saved')}</p>}
      </div>
    </form>}
    {error && <p className="inline-error" role="alert">{error}</p>}
    {!form && error && <Button onClick={() => { setError(''); setLoadAttempt(count => count + 1); }}>{t('settings.retry')}</Button>}
  </>;
}
