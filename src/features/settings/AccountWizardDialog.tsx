import { useState } from 'react';
import { Loader2 } from 'lucide-react';
import { Dialog } from '../../components/ui/Dialog';
import { Button } from '../../components/ui/Button';
import { desktopAccounts, type MailVerifyReport, type RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { OAuthDialog } from './OAuthDialog';

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
  if (!form.password) return '请填写登录密码或应用专用密码。';
  return null;
}

function toInput(form: Form, tone: Tone, id?: string | null): RealAccountInput {
  return {
    id, receiveOnly: form.receiveOnly, name: form.name.trim(), email: form.email.trim(),
    imapHost: form.imapHost.trim(), imapPort: Number(form.imapPort),
    imapSecurity: form.imapSecurity,
    smtpHost: form.smtpHost.trim(), smtpPort: Number(form.smtpPort),
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
  const [oauthOpen, setOAuthOpen] = useState(false);
  const [formError, setFormError] = useState('');
  const [report, setReport] = useState<MailVerifyReport | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState('');
  const update = (key: keyof Form) => (event: React.ChangeEvent<HTMLInputElement>) => {
    setForm(current => ({ ...current, [key]: event.target.value }));
    setFormError('');
    setSaveError('');
    setReport(null);
  };
  const valid = validate(form) === null;

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

  return <Dialog open={open} onOpenChange={value => { if (!value && !saving && !testing) onClose(); }} title={initial ? '更新 IMAP 邮箱连接' : '添加 IMAP 邮箱账号'} description="连接信息与密码只保存在本机。" className="compose-dialog">
    <fieldset className="wizard-fields" disabled={testing || saving}>
      <label><span>账号名称</span><input value={form.name} onChange={update('name')} placeholder="例如：工作邮箱" autoComplete="off" /></label>
      <label><span>邮箱地址</span><input value={form.email} onChange={update('email')} type="email" placeholder="you@example.com" autoComplete="off" /></label>
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
      <label><span>密码</span><input value={form.password} onChange={update('password')} type="password" placeholder="密码只写入系统凭据库" autoComplete="new-password" /></label>
    </fieldset>
    <p className="wizard-note">IMAP 可选 TLS 直连或必须成功升级的 STARTTLS，不支持明文登录。SMTP 使用 TLS 直连（通常 465）。验证只登录并列出文件夹，不读取正文或发送邮件；保存后可手动同步。</p>
    {report && <div className="verify-report" role="status">
      <Probe label="IMAP" probe={report.imap} extra={report.imap.ok && report.folderCount !== null ? `，发现 ${report.folderCount} 个文件夹` : undefined} />
      <Probe label="SMTP" probe={report.smtp} />
    </div>}
    {(formError || saveError) && <p className="inline-error" role="alert">{formError || saveError}</p>}
    <div className="compose-footer">
      <Button disabled={!form.name.trim() || !/.+@.+\..+/.test(form.email.trim()) || testing || saving} onClick={() => setOAuthOpen(true)}>使用 Gmail / Microsoft 授权</Button>
      <Button variant="secondary" onClick={() => void runTest()} disabled={testing || !valid}>{testing ? <Loader2 size={15} className="spin" /> : null}{testing ? '正在连接…' : '测试连接'}</Button>
      <span>保存时会再次验证，密码只写入系统凭据库。</span>
      <Button variant="primary" type="submit" onClick={() => void runSave()} disabled={saving || !valid}>{saving ? <Loader2 size={15} className="spin" /> : null}{saving ? '正在保存…' : '保存账号'}</Button>
    </div>
    {oauthOpen && <OAuthDialog account={{ ...toInput(form, tone, initial?.id), authUser: form.authUser.trim() || form.email.trim() }} onClose={() => setOAuthOpen(false)} onSaved={async () => { await onSaved(); setOAuthOpen(false); onClose(); }} />}
  </Dialog>;
}
