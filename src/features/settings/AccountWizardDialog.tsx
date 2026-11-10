import { useState } from 'react';
import { Loader2 } from 'lucide-react';
import { Dialog } from '../../components/ui/Dialog';
import { Button } from '../../components/ui/Button';
import { desktopAccounts, type MailVerifyReport, type RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';

type Tone = 'teal' | 'blue' | 'amber';

const initialForm = { name: '', email: '', imapHost: '', imapPort: '993', smtpHost: '', smtpPort: '465', authUser: '', password: '' };
type Form = typeof initialForm;

function validate(form: Form): string | null {
  if (!form.name.trim()) return '请填写账号名称。';
  if (!/.+@.+\..+/.test(form.email.trim())) return '请填写有效的邮箱地址。';
  if (!form.imapHost.trim() || !form.smtpHost.trim()) return '请填写 IMAP 与 SMTP 服务器地址。';
  const ports = [form.imapPort, form.smtpPort].map(value => Number(value));
  if (ports.some(port => !Number.isInteger(port) || port < 1 || port > 65535)) return '端口必须是 1–65535 的数字。';
  if (!form.authUser.trim()) return '请填写登录用户（通常是邮箱地址）。';
  if (!form.password) return '请填写登录密码或应用专用密码。';
  return null;
}

function toInput(form: Form, tone: Tone): RealAccountInput {
  return {
    name: form.name.trim(), email: form.email.trim(),
    imapHost: form.imapHost.trim(), imapPort: Number(form.imapPort),
    smtpHost: form.smtpHost.trim(), smtpPort: Number(form.smtpPort),
    authUser: form.authUser.trim(), avatar: (form.name.trim()[0] ?? 'A').toUpperCase(), color: tone,
  };
}

function Probe({ label, probe, extra }: { label: string; probe: MailVerifyReport['imap'] | null; extra?: string }) {
  if (!probe) return null;
  return <div className="probe">{probe.ok ? <span className="probe-ok">✓ {label}连接成功{extra}</span> : <span className="probe-fail">✕ {label}：{probe.error}</span>}</div>;
}

export function AccountWizardDialog({ open, tone, onClose, onSaved }: {
  open: boolean; tone: Tone; onClose: () => void; onSaved: () => Promise<void>;
}) {
  const [form, setForm] = useState<Form>(initialForm);
  const [formError, setFormError] = useState('');
  const [report, setReport] = useState<MailVerifyReport | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState('');
  const update = (key: keyof Form) => (event: React.ChangeEvent<HTMLInputElement>) => {
    setForm(current => ({ ...current, [key]: event.target.value }));
    setFormError('');
    setSaveError('');
  };
  const valid = validate(form) === null;

  async function runTest() {
    const problem = validate(form);
    if (problem) { setFormError(problem); return; }
    setTesting(true); setReport(null); setSaveError('');
    try {
      setReport(await desktopAccounts.verify({
        imapHost: form.imapHost.trim(), imapPort: Number(form.imapPort),
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
      await desktopAccounts.save(toInput(form, tone), form.password);
      await onSaved();
      setForm(initialForm); setReport(null);
      onClose();
    } catch (error) { setSaveError(errorMessage(error)); }
    setSaving(false);
  }

  return <Dialog open={open} onOpenChange={value => { if (!value) onClose(); }} title="添加 IMAP 邮箱账号" description="连接信息与密码只保存在本机。" className="compose-dialog">
    <div className="wizard-fields">
      <label><span>账号名称</span><input value={form.name} onChange={update('name')} placeholder="例如：工作邮箱" autoComplete="off" /></label>
      <label><span>邮箱地址</span><input value={form.email} onChange={update('email')} type="email" placeholder="you@example.com" autoComplete="off" /></label>
      <label><span>IMAP 服务器</span><input value={form.imapHost} onChange={update('imapHost')} placeholder="imap.example.com" autoComplete="off" /></label>
      <label><span>IMAP 端口</span><input value={form.imapPort} onChange={update('imapPort')} inputMode="numeric" autoComplete="off" /></label>
      <label><span>SMTP 服务器</span><input value={form.smtpHost} onChange={update('smtpHost')} placeholder="smtp.example.com" autoComplete="off" /></label>
      <label><span>SMTP 端口</span><input value={form.smtpPort} onChange={update('smtpPort')} inputMode="numeric" autoComplete="off" /></label>
      <label><span>登录用户</span><input value={form.authUser} onChange={update('authUser')} placeholder="通常与邮箱地址相同" autoComplete="off" /></label>
      <label><span>密码</span><input value={form.password} onChange={update('password')} type="password" placeholder="密码只写入系统凭据库" autoComplete="new-password" /></label>
    </div>
    <p className="wizard-note">两端均使用 TLS 直连（IMAP 993 / SMTP 465）。连接验证只做登录测试，不会读取或发送任何邮件；邮件同步将在后续版本接入。</p>
    {report && <div className="verify-report" role="status">
      <Probe label="IMAP" probe={report.imap} extra={report.imap.ok && report.folderCount !== null ? `，发现 ${report.folderCount} 个文件夹` : undefined} />
      <Probe label="SMTP" probe={report.smtp} />
    </div>}
    {(formError || saveError) && <p className="inline-error" role="alert">{formError || saveError}</p>}
    <div className="compose-footer">
      <Button variant="secondary" onClick={() => void runTest()} disabled={testing || !valid}>{testing ? <Loader2 size={15} className="spin" /> : null}{testing ? '正在连接…' : '测试连接'}</Button>
      <span>保存时会再次验证，密码只写入系统凭据库。</span>
      <Button variant="primary" type="submit" onClick={() => void runSave()} disabled={saving || !valid}>{saving ? <Loader2 size={15} className="spin" /> : null}{saving ? '正在保存…' : '保存账号'}</Button>
    </div>
  </Dialog>;
}
