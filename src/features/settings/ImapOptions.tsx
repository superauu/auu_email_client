import { useEffect, useState } from 'react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import { useImapStore } from '../../stores/imapStore';
import { errorMessage } from '../../domain/errors';
import type { ImapSettings } from '../../domain/imap';
export function ImapOptions({ accountId, name }: { accountId: string; name: string }) {
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
    try { await useImapStore.getState().save(accountId, form); setOpen(false); }
    catch (error) { setError(errorMessage(error)); }
    setSaving(false);
  }
  return <><Button variant="ghost" aria-label={`同步设置 ${name}`} onClick={() => { setForm(null); setError(''); setOpen(true); }}>同步设置</Button>
    <Dialog open={open} onOpenChange={value => { if (!saving) setOpen(value); }} title={`${name} · IMAP 同步设置`} description="设置仅应用于这个账号。">
      <div className="imap-options">
        {!form && !error && <p role="status">正在读取设置…</p>}
        {form && <>
          <label><input type="checkbox" checked={form.syncAllFolders} disabled={saving} onChange={event => setForm({ ...form, syncAllFolders: event.target.checked })} />同步全部可访问文件夹</label>
          <label><input type="checkbox" checked={form.historyEnabled} disabled={saving} onChange={event => setForm({ ...form, historyEnabled: event.target.checked })} />继续下载更早的历史邮件</label>
          <label>自动同步<select aria-label="自动同步" value={form.autoSyncSeconds} disabled={saving} onChange={event => setForm({ ...form, autoSyncSeconds: Number(event.target.value) })}><option value={0}>关闭，仅手动同步</option><option value={60}>每分钟</option><option value={300}>每 5 分钟</option><option value={900}>每 15 分钟</option></select></label>
          <p className="muted">自动同步在应用运行时生效，断线后逐步重试。未完成的同步会在重启后继续；手动停止后需重新点击同步。启用历史下载后，下次同步会继续补齐较早邮件。</p>
        </>}
        {error && <p className="inline-error" role="alert">{error}</p>}
        <div className="compose-footer"><Button disabled={saving} onClick={() => setOpen(false)}>取消</Button><Button variant="primary" disabled={!form || saving} onClick={() => void save()}>{saving ? '正在保存…' : '保存设置'}</Button></div>
      </div>
    </Dialog></>;
}
