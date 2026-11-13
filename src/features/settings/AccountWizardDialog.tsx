import { useState } from 'react';
import { ArrowLeft, Loader2 } from 'lucide-react';
import { Dialog } from '../../components/ui/Dialog';
import { Button } from '../../components/ui/Button';
import { ProviderIcon } from '../../components/ui/ProviderIcon';
import { desktopAccounts, type MailVerifyReport, type RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { PROVIDER_PRESETS, providerPreset, type ProviderId } from '../../domain/providers';
import { oauthPhaseText, useOAuthFlow } from './useOAuthFlow';

type Tone = 'teal' | 'blue' | 'amber';

const initialForm = { name: '', email: '', imapHost: '', imapPort: '993', imapSecurity: 'tls' as 'tls' | 'starttls', smtpHost: '', smtpPort: '465', authUser: '', password: '', receiveOnly: false };
type Form = typeof initialForm;

function validate(form: Form): string | null {
  if (!form.name.trim()) return '请填写账号名称。';
  if (!/.+@.+\..+/.test(form.email.trim())) return '请填写有效的邮箱地址。';
  if (!form.imapHost.trim() || !form.receiveOnly && !form.smtpHost.trim()) return '请填写 IMAP 与 SMTP 服务器地址，或选择仅收信。';
  const ports = (form.receiveOnly ? [form.imapPort] : [form.imapPort, form.smtpPort]).map(value => Number(value));
  if (ports.some(port => !Number.isInteger(port) || port < 1 || port > 65535)) return '端口必须是 1–65535 的数字。';
  if (!form.authUser.trim()) return '请填写登录用户（通常是邮箱地址）。';
  if (!form.password) return form.receiveOnly ? '请填写登录密码或授权码。' : '请填写登录密码或应用专用密码。';
  return null;
}

function toInput(form: Form, tone: Tone, id?: string | null): RealAccountInput {
  return {
    id, receiveOnly: form.receiveOnly, name: form.name.trim(), email: form.email.trim(),
    imapHost: form.imapHost.trim(), imapPort: Number(form.imapPort) || 993,
    imapSecurity: form.imapSecurity,
    smtpHost: form.smtpHost.trim(), smtpPort: Number(form.smtpPort) || 465,
    authUser: form.authUser.trim(), avatar: (form.name.trim()[0] ?? 'A').toUpperCase(), color: tone,
  };
}

function Probe({ label, probe, extra }: { label: string; probe: MailVerifyReport['imap'] | null; extra?: string }) {
  if (!probe) return null;
  if (probe.skipped) return <div className="probe"><span className="muted">{label} 未验证（仅收信）</span></div>;
  return <div className="probe">{probe.ok ? <span className="probe-ok">✓ {label}连接成功{extra}</span> : <span className="probe-fail">✕ {label}：{probe.error}</span>}</div>;
}

export function AccountWizardDialog({ open, tone, onClose, onSaved, initial }: {
  open: boolean; tone: Tone; onClose: () => void; onSaved: () => Promise<void>; initial?: RealAccountInput;
}) {
  const [form, setForm] = useState<Form>(() => initial ? { name: initial.name, email: initial.email, imapHost: initial.imapHost, imapPort: String(initial.imapPort), imapSecurity: initial.imapSecurity ?? 'tls', smtpHost: initial.smtpHost, smtpPort: String(initial.smtpPort), authUser: initial.authUser, password: '', receiveOnly: initial.receiveOnly ?? false } : initialForm);
  // 编辑模式跳过提供商选择，直接进入 IMAP 表单。
  const [providerId, setProviderId] = useState<ProviderId | null>(initial ? 'imap' : null);
  const [formError, setFormError] = useState('');
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
  const emailReady = !!form.name.trim() && /.+@.+\..+/.test(form.email.trim());
  const oauthSaved = oauth.flow?.state === 'saved';

  function chooseProvider(id: ProviderId) {
    const target = providerPreset(id);
    if (!target.available) return;
    setProviderId(id);
    setFormError('');
    setSaveError('');
    setReport(null);
    if (target.endpoints) {
      setForm(current => ({ ...current, imapHost: target.endpoints!.imapHost, imapPort: String(target.endpoints!.imapPort), imapSecurity: target.endpoints!.imapSecurity, smtpHost: target.endpoints!.smtpHost, smtpPort: String(target.endpoints!.smtpPort), authUser: current.authUser.trim() || current.email.trim() }));
    }
  }

  function backToProviders() {
    setProviderId(null);
    setFormError('');
    setSaveError('');
    setReport(null);
  }

  async function runTest() {
    const problem = validate(form);
    if (problem) { setFormError(problem); return; }
    setTesting(true); setReport(null); setSaveError('');
    try {
      setReport(await desktopAccounts.verify({
        receiveOnly: form.receiveOnly,
        imapHost: form.imapHost.trim(), imapPort: Number(form.imapPort),
        imapSecurity: form.imapSecurity,
        smtpHost: form.smtpHost.trim(), smtpPort: Number(form.smtpPort), authUser: form.authUser.trim(),
      }, form.password));
    } catch (error) { setReport(null); setSaveError(errorMessage(error)); }
    setTesting(false);
  }

  async function runSave() {
    const problem = validate(form);
    if (problem) { setFormError(problem); return; }
    setSaving(true); setSaveError('');
    try {
      await desktopAccounts.save(toInput(form, tone, initial?.id), form.password);
      await onSaved();
      setForm(initialForm); setReport(null);
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
    } catch (error) { oauth.setError(`账号已保存，刷新工作区失败：${errorMessage(error)}`); }
  }

  const providerStep = !initial && !providerId;
  const title = initial ? '更新 IMAP 邮箱连接' : providerStep ? '添加邮箱账号' : preset ? `添加 ${preset.label} 账号` : '添加邮箱账号';
  const description = providerStep ? '选择邮箱提供商，之后只需填写名称和地址。' : oauthOnly ? '授权在系统浏览器完成，凭据只保存在本机。' : '连接信息与密码只保存在本机。';
  const dialogBusy = testing || saving || oauth.running;

  return <Dialog open={open} onOpenChange={value => { if (!value && !dialogBusy) onClose(); }} title={title} description={description} className="compose-dialog">
    {providerStep && <>
      <div className="provider-grid">{PROVIDER_PRESETS.map(item => (
        <button key={item.id} className="provider-card" disabled={!item.available} aria-label={item.available ? `添加 ${item.label} 账号` : `${item.label}，即将支持`} title={item.available ? undefined : '该平台的 OAuth 应用注册完成后开放'} onClick={() => chooseProvider(item.id)}>
          <span className="provider-icon-slot"><ProviderIcon provider={item.id} /></span>
          <span className="provider-card-title">{item.label}{!item.available && <span className="soon-tag">即将支持</span>}</span>
          <small>{item.description}</small>
        </button>
      ))}</div>
      <p className="provider-step-note">Gmail 通过系统浏览器授权，无需填写服务器信息；QQ / 163 使用授权码登录；其他邮箱手动填写 IMAP / SMTP。密码只写入系统凭据库。</p>
    </>}
    {!providerStep && preset && <>
      {oauthOnly && <div className="oauth-hero">
        <span className="provider-icon-slot provider-icon-slot-lg"><ProviderIcon provider="gmail" /></span>
        <div>
          <h3>使用 Google 账号授权</h3>
          <p>只需名称和地址用于识别账号。授权在系统浏览器完成，AuuMail 使用内置的公共应用标识（无需注册或填写 Client ID），只请求邮件读取权限；刷新令牌保存到系统凭据库，不会离开本机。</p>
        </div>
      </div>}
      <fieldset className="wizard-fields" disabled={dialogBusy}>
        <label><span>账号名称</span><input value={form.name} onChange={update('name')} placeholder="例如：工作邮箱" autoComplete="off" /></label>
        <label><span>邮箱地址</span><input value={form.email} onChange={update('email')} type="email" placeholder={oauthOnly ? 'you@gmail.com' : 'you@example.com'} autoComplete="off" /></label>
        {!oauthOnly && <>
          <label><span>IMAP 服务器</span><input value={form.imapHost} onChange={update('imapHost')} placeholder="imap.example.com" autoComplete="off" /></label>
          <label><span>IMAP 端口</span><input value={form.imapPort} onChange={update('imapPort')} inputMode="numeric" autoComplete="off" /></label>
          <label><span>IMAP 加密</span><select value={form.imapSecurity} disabled={testing || saving} onChange={event => {
            const security = event.target.value === 'starttls' ? 'starttls' : 'tls';
            setForm(current => ({ ...current, imapSecurity: security, imapPort: ['993', '143'].includes(current.imapPort) ? security === 'starttls' ? '143' : '993' : current.imapPort }));
            setReport(null);
          }}><option value="tls">TLS 直连（通常 993）</option><option value="starttls">STARTTLS（通常 143）</option></select></label>
          <label><span>仅收信</span><input type="checkbox" checked={form.receiveOnly} onChange={event => { setForm({ ...form, receiveOnly: event.target.checked }); setReport(null); }} />不验证 SMTP</label>
          <label><span>SMTP 服务器</span><input disabled={form.receiveOnly} value={form.smtpHost} onChange={update('smtpHost')} placeholder="smtp.example.com" autoComplete="off" /></label>
          <label><span>SMTP 端口</span><input disabled={form.receiveOnly} value={form.smtpPort} onChange={update('smtpPort')} inputMode="numeric" autoComplete="off" /></label>
          <label><span>登录用户</span><input value={form.authUser} onChange={update('authUser')} placeholder="通常与邮箱地址相同" autoComplete="off" /></label>
          <label><span>{preset.authHint ? '授权码' : '密码'}</span><input value={form.password} onChange={update('password')} type="password" placeholder={preset.authHint ? preset.authHint : '密码只写入系统凭据库'} autoComplete="new-password" /></label>
        </>}
      </fieldset>
      {!oauthOnly && <p className="wizard-note">{preset.authHint ? `${preset.authHint} ` : ''}IMAP 可选 TLS 直连或必须成功升级的 STARTTLS，不支持明文登录。SMTP 使用 TLS 直连（通常 465）。验证只登录并列出文件夹，不读取正文或发送邮件；保存后可手动同步。</p>}
      {report && <div className="verify-report" role="status">
        <Probe label="IMAP" probe={report.imap} extra={report.imap.ok && report.folderCount !== null ? `，发现 ${report.folderCount} 个文件夹` : undefined} />
        <Probe label="SMTP" probe={report.smtp} />
      </div>}
      {oauth.flow && <p className="oauth-status" role="status">{oauthPhaseText[oauth.flow.state]}{oauth.flow.folderCount !== null ? `发现 ${oauth.flow.folderCount} 个文件夹。` : ''}</p>}
      {(oauth.error || oauth.flow?.error) && <p className="inline-error" role="alert">{oauth.error || oauth.flow?.error}</p>}
      {(formError || saveError) && <p className="inline-error" role="alert">{formError || saveError}</p>}
      <div className="compose-footer">
        {oauthOnly ? <>
          <Button disabled={oauth.running} onClick={backToProviders}><ArrowLeft size={15} />返回</Button>
          <span>{oauthSaved ? '账号已保存，刷新后即可在工作区使用。' : '授权完成后账号自动保存。'}</span>
          {oauthSaved
            ? <Button variant="primary" onClick={() => void finishSaved()}>刷新账号列表</Button>
            : oauth.running
              ? <Button variant="secondary" onClick={() => void oauth.stop()}>取消授权</Button>
              : <Button variant="primary" disabled={!emailReady} onClick={() => void startGmailAuth()}>{oauth.flow?.state === 'failed' || oauth.flow?.state === 'cancelled' ? '重新授权' : '在浏览器中授权'}</Button>}
        </> : <>
          {!initial && <Button onClick={backToProviders}><ArrowLeft size={15} />返回</Button>}
          <Button variant="secondary" onClick={() => void runTest()} disabled={testing || !valid}>{testing ? <Loader2 size={15} className="spin" /> : null}{testing ? '正在连接…' : '测试连接'}</Button>
          <span>保存时会再次验证，密码只写入系统凭据库。</span>
          <Button variant="primary" type="submit" onClick={() => void runSave()} disabled={saving || !valid}>{saving ? <Loader2 size={15} className="spin" /> : null}{saving ? '正在保存…' : '保存账号'}</Button>
        </>}
      </div>
    </>}
  </Dialog>;
}
