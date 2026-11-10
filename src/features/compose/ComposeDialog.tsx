import { useRef, useState } from 'react';
import { Save, Send } from 'lucide-react';
import { Dialog } from '../../components/ui/Dialog';
import { Button } from '../../components/ui/Button';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import type { MailDraft } from '../../domain/mail';
import { errorMessage } from '../../domain/errors';

export function ComposeDialog({ initial, onClose }: { initial: MailDraft; onClose: () => void }) {
  const accounts = useWorkspaceStore(s => s.accounts);
  const saveDraft = useWorkspaceStore(s => s.saveDraft);
  const [draft, setDraft] = useState(initial);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const dirty = useRef(false);
  const [discardPrompt, setDiscardPrompt] = useState(false);
  const update = (changes: Partial<MailDraft>) => { setDraft(value => ({ ...value, ...changes })); dirty.current = true; setSaved(false); };
  async function save(closeAfter: boolean) {
    setSaving(true); setError('');
    try {
      await saveDraft({ ...draft, updatedAt: new Date().toISOString() });
      dirty.current = false; setSaved(true); setDiscardPrompt(false);
      if (closeAfter) onClose();
    } catch (error) { setError(errorMessage(error)); }
    finally { setSaving(false); }
  }
  function close() {
    if (saving) return;
    if (dirty.current) setDiscardPrompt(true);
    else onClose();
  }
  return <Dialog open onOpenChange={open => { if (!open) close(); }} title={initial.replyToId ? '回复邮件' : '写一封邮件'} description="演示工作区 · 草稿保存在本机，尚未连接真实邮箱。" className="compose-dialog">
    <form onSubmit={event => { event.preventDefault(); void save(false); }}>
      <div className="compose-fields">
        <label><span id="compose-from-label">发件人</span><select aria-labelledby="compose-from-label" value={draft.accountId} onChange={e => update({ accountId: e.target.value })} disabled={saving}>{accounts.map(a => <option key={a.id} value={a.id}>{a.name} &lt;{a.email}&gt;</option>)}</select></label>
        <label><span>收件人</span><input value={draft.to} maxLength={1000} onChange={e => update({ to: e.target.value })} placeholder="name@example.com" disabled={saving} /></label>
        <label><span>主题</span><input value={draft.subject} maxLength={1000} onChange={e => update({ subject: e.target.value })} placeholder="给这封邮件一个主题" disabled={saving} /></label>
      </div>
      <textarea className="compose-body" aria-label="邮件正文" value={draft.bodyText} maxLength={50000} onChange={e => update({ bodyText: e.target.value })} placeholder="从一句问候开始…" disabled={saving} />
      {error && <p className="inline-error" role="alert">{error}</p>}
      {discardPrompt && <div className="discard-prompt" role="alert"><span>还有未保存的修改。</span><Button onClick={() => void save(true)} disabled={saving}>保存并关闭</Button><Button variant="ghost" onClick={onClose}>放弃修改</Button><Button variant="ghost" onClick={() => setDiscardPrompt(false)}>继续编辑</Button></div>}
      <footer className="compose-footer"><span role="status">{saved ? '草稿已保存' : '尚未连接邮箱，暂不能发送'}</span><Button type="submit" disabled={saving}><Save size={15} />{saving ? '正在保存…' : '保存草稿'}</Button><Button variant="primary" disabled title="请先连接真实邮箱"><Send size={15} />发送</Button></footer>
    </form>
  </Dialog>;
}
