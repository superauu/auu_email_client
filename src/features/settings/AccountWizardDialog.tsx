import { useState } from 'react';
import { ArrowLeft, Loader2 } from 'lucide-react';
import { Dialog } from '../../components/ui/Dialog';
import { Button } from '../../components/ui/Button';
import { ProviderIcon } from '../../components/ui/ProviderIcon';
import { desktopAccounts, type MailVerifyReport, type RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { PROVIDER_PRESETS, providerPreset, providerPresetLabel, providerPresetDescription, providerPresetAuthHint, type ProviderId } from '../../domain/providers';
import { oauthPhaseText, useOAuthFlow } from './useOAuthFlow';
import { t as coreT } from '../../i18n/core';
import { useI18n } from '../../i18n/react';

type Tone = 'teal' | 'blue' | 'amber';

const initialForm = { name: '', email: '', imapHost: '', imapPort: '993', imapSecurity: 'tls' as 'tls' | 'starttls', smtpHost: '', smtpPort: '465', authUser: '', password: '', receiveOnly: false };
type Form = typeof initialForm;

function validate(form: Form): string | null {
  if (!form.name.trim()) return coreT('wizard.errName');
  if (!/.+@.+\..+/.test(form.email.trim())) return coreT('wizard.errEmail');
  if (!form.imapHost.trim() || !form.receiveOnly && !form.smtpHost.trim()) return coreT('wizard.errHosts');
  const ports = (form.receiveOnly ? [form.imapPort] : [form.imapPort, form.smtpPort]).map(value => Number(value));
  if (ports.some(port => !Number.isInteger(port) || port < 1 || port > 65535)) return coreT('wizard.errPort');
  if (!form.authUser.trim()) return coreT('wizard.errAuthUser');
  if (!form.password) return form.receiveOnly ? coreT('wizard.errPasswordReceive') : coreT('wizard.errPassword');
  return null;
}

/** 字段级校验：返回每个输入框是否无效，供 aria-invalid 映射。 */
function fieldErrors(form: Form): Record<string, boolean> {
  const imapPort = Number(form.imapPort);
  const smtpPort = Number(form.smtpPort);
  const portBad = (value: number) => !Number.isInteger(value) || value < 1 || value > 65535;
  return {
    name: !form.name.trim(),
    email: !/.+@.+\..+/.test(form.email.trim()),
    imapHost: !form.imapHost.trim(),
    imapPort: portBad(imapPort),
    smtpHost: !form.receiveOnly && !form.smtpHost.trim(),
    smtpPort: !form.receiveOnly && portBad(smtpPort),
    authUser: !form.authUser.trim(),
    password: !form.password,
  };
}

/** 单字段错误对应的说明文案键（配合 aria-describedby 指向输入框旁的错误文本）。 */
function fieldErrorKey(form: Form, key: string): 'wizard.fieldRequired' | 'wizard.fieldEmailInvalid' | 'wizard.fieldPortInvalid' | null {
  const errors = fieldErrors(form);
  if (!errors[key]) return null;
  if (key === 'email') return 'wizard.fieldEmailInvalid';
  if (key === 'imapPort' || key === 'smtpPort') return 'wizard.fieldPortInvalid';
  return 'wizard.fieldRequired';
}

function toInput(form: Form, tone: Tone, id?: string | null, provider: 'imap' | 'pop3' = 'imap'): RealAccountInput {
  return {
    id, provider, receiveOnly: form.receiveOnly, name: form.name.trim(), email: form.email.trim(),
    imapHost: form.imapHost.trim(), imapPort: Number(form.imapPort) || 993,
    imapSecurity: provider === 'pop3' ? 'tls' : form.imapSecurity,
    smtpHost: form.smtpHost.trim(), smtpPort: Number(form.smtpPort) || 465,
    authUser: form.authUser.trim(), avatar: (form.name.trim()[0] ?? 'A').toUpperCase(), color: tone,
  };
}

function Probe({ label, probe, extra }: { label: string; probe: MailVerifyReport['imap'] | null; extra?: string }) {
  const t = useI18n();
  if (!probe) return null;
  if (probe.skipped) return <div className="probe"><span className="muted">{t('wizard.probeSkipped', { label })}</span></div>;
  return <div className="probe">{probe.ok ? <span className="probe-ok">{t('wizard.probeOk', { label, extra: extra ?? '' })}</span> : <span className="probe-fail">{t('wizard.probeFail', { label, error: probe.error ?? '' })}</span>}</div>;
}

export function AccountWizardDialog({ open, tone, onClose, onSaved, initial }: {
  open: boolean; tone: Tone; onClose: () => void; onSaved: () => Promise<void>; initial?: RealAccountInput;
}) {
  const t = useI18n();
  const [form, setForm] = useState<Form>(() => initial ? { name: initial.name, email: initial.email, imapHost: initial.imapHost, imapPort: String(initial.imapPort), imapSecurity: initial.imapSecurity ?? 'tls', smtpHost: initial.smtpHost, smtpPort: String(initial.smtpPort), authUser: initial.authUser, password: '', receiveOnly: initial.receiveOnly ?? false } : initialForm);
  // 编辑模式保留已保存的协议，不能将 POP3 原地重配成 IMAP。
  const [providerId, setProviderId] = useState<ProviderId | null>(initial ? initial.provider === 'pop3' ? 'pop3' : 'imap' : null);
  const [formError, setFormError] = useState('');
  // 字段级校验的可见性：仅在用户触碰过（或提交失败）后标记 aria-invalid，避免打开即全红。
  const [touched, setTouched] = useState<Record<string, boolean>>({});
  const touch = (key: string) => () => setTouched(current => current[key] ? current : { ...current, [key]: true });
  const markAllTouched = () => setTouched({ name: true, email: true, imapHost: true, imapPort: true, smtpHost: true, smtpPort: true, authUser: true, password: true });
  const resetTouched = () => setTouched({});
  const [report, setReport] = useState<MailVerifyReport | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState('');
  const oauth = useOAuthFlow();
  const update = (key: keyof Form) => (event: React.ChangeEvent<HTMLInputElement>) => {
    const value = event.target.value;
    setForm(current => {
      const next = { ...current, [key]: value };
      if (key === 'email' && (!current.authUser.trim() || current.authUser.trim() === current.email.trim())) next.authUser = value.trim();
      return next;
    });
    setFormError('');
    setSaveError('');
    setReport(null);
  };
  const preset = providerId ? providerPreset(providerId) : null;
  const oauthOnly = preset?.oauth === true && !initial;
  const valid = validate(form) === null;
  const errors = fieldErrors(form);
  // 触控（或提交失败）后才对外呈现字段级无效态与错误说明。
  const shown = (key: string) => Boolean(touched[key] && errors[key]);
  const emailReady = !!form.name.trim() && /.+@.+\..+/.test(form.email.trim());
  const oauthSaved = oauth.flow?.state === 'saved';

  function chooseProvider(id: ProviderId) {
    const target = providerPreset(id);
    if (!target.available) return;
    setProviderId(id);
    setFormError('');
    setSaveError('');
    setReport(null);
    resetTouched();
    if (target.endpoints) {
      setForm(current => ({ ...current, imapHost: target.endpoints!.imapHost, imapPort: String(target.endpoints!.imapPort), imapSecurity: target.endpoints!.imapSecurity, smtpHost: target.endpoints!.smtpHost, smtpPort: String(target.endpoints!.smtpPort), authUser: current.authUser.trim() || current.email.trim() }));
    }
    if (id === 'pop3') {
      setForm(current => ({ ...current, imapSecurity: 'tls', imapPort: ['993', '143'].includes(current.imapPort) ? '995' : current.imapPort }));
    }
  }

  function backToProviders() {
    setProviderId(null);
    setFormError('');
    setSaveError('');
    setReport(null);
    resetTouched();
  }

  async function runTest() {
    const problem = validate(form);
    if (problem) { setFormError(problem); markAllTouched(); return; }
    setTesting(true); setReport(null); setSaveError('');
    try {
      setReport(await desktopAccounts.verify({
        provider: providerId === 'pop3' ? 'pop3' : 'imap',
        receiveOnly: form.receiveOnly,
        imapHost: form.imapHost.trim(), imapPort: Number(form.imapPort),
        imapSecurity: providerId === 'pop3' ? 'tls' : form.imapSecurity,
        smtpHost: form.smtpHost.trim(), smtpPort: Number(form.smtpPort), authUser: form.authUser.trim(),
      }, form.password));
    } catch (error) { setReport(null); setSaveError(errorMessage(error)); }
    setTesting(false);
  }

  async function runSave() {
    const problem = validate(form);
    if (problem) { setFormError(problem); markAllTouched(); return; }
    setSaving(true); setSaveError('');
    try {
      await desktopAccounts.save(toInput(form, tone, initial?.id, providerId === 'pop3' ? 'pop3' : 'imap'), form.password);
      await onSaved();
      setForm(initialForm); setReport(null); resetTouched();
      onClose();
    } catch (error) { setSaveError(errorMessage(error)); }
    setSaving(false);
  }

  async function startGmailAuth() {
    if (!emailReady) return;
    await oauth.begin({ ...toInput(form, tone, initial?.id), authUser: form.authUser.trim() || form.email.trim() }, { provider: 'google', clientId: '', tenant: 'common' });
  }

  async function finishSaved() {
    try {
      oauth.savedRef.current = true;
      await onSaved();
      onClose();
    } catch (error) { oauth.setError(t('wizard.savedRefreshFailed', { error: errorMessage(error) })); }
  }

  const providerStep = !initial && !providerId;
  const title = initial ? t('wizard.titleUpdate', { protocol: providerId === 'pop3' ? 'POP3' : 'IMAP' }) : providerStep || !preset ? t('wizard.titleAdd') : t('wizard.titleAddProvider', { provider: providerPresetLabel(preset.id) });
  const description = providerStep ? t('wizard.descProviderStep') : oauthOnly ? t('wizard.descOauth') : t('wizard.descManual');
  const dialogBusy = testing || saving || oauth.running;

  return <Dialog open={open} onOpenChange={value => { if (!value && !dialogBusy) onClose(); }} title={title} description={description} className="compose-dialog">
    {providerStep && <>
      <div className="provider-grid">{PROVIDER_PRESETS.map(item => (
        <button key={item.id} className="provider-card" disabled={!item.available} aria-label={item.available ? t('wizard.titleAddProvider', { provider: providerPresetLabel(item.id) }) : t('wizard.providerSoonAria', { provider: providerPresetLabel(item.id) })} title={item.available ? undefined : t('wizard.soonTitle')} onClick={() => chooseProvider(item.id)}>
          <span className="provider-icon-slot"><ProviderIcon provider={item.id} /></span>
          <span className="provider-card-title">{providerPresetLabel(item.id)}{!item.available && <span className="soon-tag">{t('wizard.soonTag')}</span>}</span>
          <small>{providerPresetDescription(item.id)}</small>
        </button>
      ))}</div>
      <p className="provider-step-note">{t('wizard.stepNote')}</p>
    </>}
    {!providerStep && preset && <>
      {oauthOnly && <div className="oauth-hero">
        <span className="provider-icon-slot provider-icon-slot-lg"><ProviderIcon provider="gmail" /></span>
        <div>
          <h3>{t('wizard.oauthHeading')}</h3>
          <p>{t('wizard.oauthIntro')}</p>
        </div>
      </div>}
      <fieldset className="wizard-fields" disabled={dialogBusy}>
        <label><span>{t('wizard.nameLabel')}</span><input value={form.name} onChange={update('name')} onBlur={touch('name')} placeholder={t('wizard.namePlaceholder')} autoComplete="off" aria-invalid={shown('name') ? 'true' : undefined} aria-describedby={shown('name') ? 'wizard-err-name' : undefined} />{shown('name') && <span className="field-error" id="wizard-err-name">{t(fieldErrorKey(form, 'name')!)}</span>}</label>
        <label><span>{t('wizard.emailLabel')}</span><input value={form.email} onChange={update('email')} onBlur={touch('email')} type="email" placeholder={oauthOnly ? 'you@gmail.com' : 'you@example.com'} autoComplete="off" aria-invalid={shown('email') ? 'true' : undefined} aria-describedby={shown('email') ? 'wizard-err-email' : undefined} />{shown('email') && <span className="field-error" id="wizard-err-email">{t(fieldErrorKey(form, 'email')!)}</span>}</label>
        {!oauthOnly && <>
          <label><span>{t('wizard.serverLabel', { protocol: providerId === 'pop3' ? 'POP3' : 'IMAP' })}</span><input value={form.imapHost} onChange={update('imapHost')} onBlur={touch('imapHost')} placeholder={providerId === 'pop3' ? 'pop3.example.com' : 'imap.example.com'} autoComplete="off" aria-invalid={shown('imapHost') ? 'true' : undefined} aria-describedby={shown('imapHost') ? 'wizard-err-imapHost' : undefined} />{shown('imapHost') && <span className="field-error" id="wizard-err-imapHost">{t(fieldErrorKey(form, 'imapHost')!)}</span>}</label>
          <label><span>{t('wizard.portLabel', { protocol: providerId === 'pop3' ? 'POP3' : 'IMAP' })}</span><input value={form.imapPort} onChange={update('imapPort')} onBlur={touch('imapPort')} inputMode="numeric" autoComplete="off" aria-invalid={shown('imapPort') ? 'true' : undefined} aria-describedby={shown('imapPort') ? 'wizard-err-imapPort' : undefined} />{shown('imapPort') && <span className="field-error" id="wizard-err-imapPort">{t(fieldErrorKey(form, 'imapPort')!)}</span>}</label>
          <label><span>{t('wizard.securityLabel', { protocol: providerId === 'pop3' ? 'POP3' : 'IMAP' })}</span><select value={form.imapSecurity} disabled={testing || saving} onChange={event => {
            const security = event.target.value === 'starttls' ? 'starttls' : 'tls';
            setForm(current => ({ ...current, imapSecurity: security, imapPort: ['993', '143'].includes(current.imapPort) ? security === 'starttls' ? '143' : '993' : current.imapPort }));
            setReport(null);
          }}><option value="tls">{t('wizard.tlsOption', { port: providerId === 'pop3' ? '995' : '993' })}</option>{providerId !== 'pop3' && <option value="starttls">{t('wizard.starttlsOption')}</option>}</select></label>
          <label><span>{t('wizard.receiveOnlyLabel')}</span><input type="checkbox" checked={form.receiveOnly} onChange={event => { setForm({ ...form, receiveOnly: event.target.checked }); setReport(null); }} />{t('wizard.receiveOnlyHint')}</label>
          <label><span>{t('wizard.serverLabel', { protocol: 'SMTP' })}</span><input disabled={form.receiveOnly} value={form.smtpHost} onChange={update('smtpHost')} onBlur={touch('smtpHost')} placeholder="smtp.example.com" autoComplete="off" aria-invalid={shown('smtpHost') ? 'true' : undefined} aria-describedby={shown('smtpHost') ? 'wizard-err-smtpHost' : undefined} />{shown('smtpHost') && <span className="field-error" id="wizard-err-smtpHost">{t(fieldErrorKey(form, 'smtpHost')!)}</span>}</label>
          <label><span>{t('wizard.portLabel', { protocol: 'SMTP' })}</span><input disabled={form.receiveOnly} value={form.smtpPort} onChange={update('smtpPort')} onBlur={touch('smtpPort')} inputMode="numeric" autoComplete="off" aria-invalid={shown('smtpPort') ? 'true' : undefined} aria-describedby={shown('smtpPort') ? 'wizard-err-smtpPort' : undefined} />{shown('smtpPort') && <span className="field-error" id="wizard-err-smtpPort">{t(fieldErrorKey(form, 'smtpPort')!)}</span>}</label>
          <label><span>{t('wizard.authUserLabel')}</span><input value={form.authUser} onChange={update('authUser')} onBlur={touch('authUser')} placeholder={t('wizard.authUserPlaceholder')} autoComplete="off" aria-invalid={shown('authUser') ? 'true' : undefined} aria-describedby={shown('authUser') ? 'wizard-err-authUser' : undefined} />{shown('authUser') && <span className="field-error" id="wizard-err-authUser">{t(fieldErrorKey(form, 'authUser')!)}</span>}</label>
          <label><span>{providerPresetAuthHint(preset.id) ? t('wizard.authCodeLabel') : t('wizard.passwordLabel')}</span><input value={form.password} onChange={update('password')} onBlur={touch('password')} type="password" placeholder={providerPresetAuthHint(preset.id) ?? t('wizard.passwordPlaceholder')} autoComplete="new-password" aria-invalid={shown('password') ? 'true' : undefined} aria-describedby={shown('password') ? 'wizard-err-password' : undefined} />{shown('password') && <span className="field-error" id="wizard-err-password">{t(fieldErrorKey(form, 'password')!)}</span>}</label>
        </>}
      </fieldset>
      {!oauthOnly && <p className="wizard-note">{providerPresetAuthHint(preset.id) ? `${providerPresetAuthHint(preset.id)} ` : ''}{providerId === 'pop3' ? t('wizard.notePop3') : t('wizard.noteImap')}{t('wizard.noteSmtp')}</p>}
      {report && <div className="verify-report" role="status">
        <Probe label={providerId === 'pop3' ? 'POP3' : 'IMAP'} probe={report.imap} extra={report.imap.ok && report.folderCount !== null ? t('wizard.probeExtra', { count: report.folderCount, unit: providerId === 'pop3' ? t('wizard.unitMessages') : t('wizard.unitFolders') }) : undefined} />
        <Probe label="SMTP" probe={report.smtp} />
      </div>}
      {oauth.flow && <p className="oauth-status" role="status">{oauthPhaseText(oauth.flow.state)}{oauth.flow.folderCount !== null ? t('wizard.oauthFolders', { count: oauth.flow.folderCount }) : ''}</p>}
      {(oauth.error || oauth.flow?.error) && <p className="inline-error" role="alert">{oauth.error || oauth.flow?.error}</p>}
      {(formError || saveError) && <p className="inline-error" role="alert">{formError || saveError}</p>}
      <div className="compose-footer">
        {oauthOnly ? <>
          <Button disabled={oauth.running} onClick={backToProviders}><ArrowLeft size={15} />{t('wizard.back')}</Button>
          <span>{oauthSaved ? t('wizard.oauthSavedNote') : t('wizard.oauthAutoNote')}</span>
          {oauthSaved
            ? <Button variant="primary" onClick={() => void finishSaved()}>{t('wizard.refreshAccounts')}</Button>
            : oauth.running
              ? <Button variant="secondary" onClick={() => void oauth.stop()}>{t('wizard.cancelAuth')}</Button>
              : <Button variant="primary" disabled={!emailReady} onClick={() => void startGmailAuth()}>{oauth.flow?.state === 'failed' || oauth.flow?.state === 'cancelled' ? t('settings.reauthorize') : t('wizard.authorizeBrowser')}</Button>}
        </> : <>
          {!initial && <Button onClick={backToProviders}><ArrowLeft size={15} />{t('wizard.back')}</Button>}
          <Button variant="secondary" onClick={() => void runTest()} disabled={testing || !valid}>{testing ? <Loader2 size={15} className="spin" /> : null}{testing ? t('wizard.testing') : t('wizard.testConnection')}</Button>
          <span>{t('wizard.saveHint')}</span>
          <Button variant="primary" type="submit" onClick={() => void runSave()} disabled={saving || !valid}>{saving ? <Loader2 size={15} className="spin" /> : null}{saving ? t('wizard.saving') : t('wizard.saveAccount')}</Button>
        </>}
      </div>
    </>}
  </Dialog>;
}
