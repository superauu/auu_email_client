import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Loader2, Save, Send } from 'lucide-react';
import { Dialog } from '../../components/ui/Dialog';
import { Button } from '../../components/ui/Button';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import type { MailDraft } from '../../domain/mail';
import { errorMessage } from '../../domain/errors';
import { desktop, isDesktop } from '../../platform/desktop';
import { DraftSession } from './DraftSession';

export function ComposeDialog({ initial, onClose }: { initial: MailDraft; onClose: () => void }) {
  const accounts = useWorkspaceStore(s => s.accounts);
  const saveDraft = useWorkspaceStore(s => s.saveDraft);
  const sendDraft = useWorkspaceStore(s => s.sendDraft);
  const [session] = useState(() => new DraftSession(initial, saveDraft));
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { draft } = state;
  const [closeTarget, setCloseTarget] = useState<'dialog' | 'window' | null>(null);
  const [platformError, setPlatformError] = useState('');
  const [sendNotice, setSendNotice] = useState('');
  const [sending, setSending] = useState(false);
  const allowWindowClose = useRef(false);
  const account = accounts.find(a => a.id === draft.accountId);
  const canSend = isDesktop() && account?.capabilities.send === true;
  const sendBlockReason = !isDesktop()
    ? '浏览器演示不能发送邮件'
    : !account || account.status === 'demo'
      ? '演示账号不能发送邮件'
      : account.capabilities.send
        ? ''
        : '当前账号尚未开放发送（仅收信账号，或服务商未授权 SMTP 发送）';

  useEffect(() => {
    function beforeUnload(event: BeforeUnloadEvent) {
      if (!allowWindowClose.current && session.needsProtection()) { event.preventDefault(); event.returnValue = ''; }
    }
    window.addEventListener('beforeunload', beforeUnload);
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void desktop.guardWindowClose(() => !allowWindowClose.current && session.needsProtection(), () => {
      session.pause(); setCloseTarget('window');
    }).then(cleanup => { if (disposed) cleanup(); else unlisten = cleanup; }).catch(error => {
      if (!disposed) setPlatformError(`未能启用窗口关闭保护：${errorMessage(error)}`);
    });
    return () => { disposed = true; unlisten?.(); session.cancelScheduledSave(); window.removeEventListener('beforeunload', beforeUnload); };
  }, [session]);

  function requestClose() {
    if (session.needsProtection()) { session.pause(); setCloseTarget('dialog'); }
    else { session.cancelScheduledSave(); onClose(); }
  }
  async function finishClose() {
    session.cancelScheduledSave();
    if (closeTarget === 'window') {
      allowWindowClose.current = true;
      try { await desktop.windowAction('close'); }
      catch (error) { allowWindowClose.current = false; setPlatformError(errorMessage(error)); }
    } else onClose();
  }
  async function save(closeAfter: boolean) {
    setPlatformError('');
    if (await session.flush()) {
      if (closeAfter) await finishClose();
      else { setCloseTarget(null); session.resume(); }
    }
  }
  async function send() {
    setPlatformError('');
    setSending(true);
    try {
      // 先保证草稿落盘并拿到当前 revision，再登记发送队列（docs/SENDING.md）。
      if (!(await session.flush())) throw new Error('草稿尚未保存成功，请稍后重试。');
      const entry = await sendDraft(draft.id, session.currentRevision);
      setSendNotice(entry.state === 'queued' ? '已进入发送队列；发送结果可在设置 → 邮箱账号查看，结果不明时不会自动重发。' : `发送任务状态：${entry.state}`);
      setCloseTarget(null);
      session.resume();
    } catch (error) { setPlatformError(errorMessage(error)); }
    finally { setSending(false); }
  }
  const status = state.error ? '保存失败，内容仍保留在窗口中' : state.saving ? '正在保存草稿…' : state.dirty ? '尚未保存，将自动保存' : state.saved ? '草稿已保存' : '修改后将自动保存草稿';
  return <Dialog open onOpenChange={open => { if (!open) requestClose(); }} title={initial.replyToId ? /^转发[:：]/.test(initial.subject) ? '转发邮件' : '回复邮件' : '写一封邮件'} description={canSend ? '草稿保存在本机；发送经 SMTP 完成，结果在设置 → 邮箱账号查看。' : isDesktop() ? '草稿自动保存在这台设备上，不会上传或发送。' : '浏览器演示 · 草稿自动保存在本机。'} className="compose-dialog">
    <form onSubmit={event => { event.preventDefault(); void save(false); }}>
      <div className="compose-fields">
        <label><span id="compose-from-label">发件人</span><select aria-labelledby="compose-from-label" value={draft.accountId} onChange={e => session.update({ accountId: e.target.value })} disabled={!!draft.replyToId} title={draft.replyToId ? '回复草稿使用原邮件所属账号' : undefined}>{accounts.map(a => <option key={a.id} value={a.id}>{a.name} &lt;{a.email}&gt;</option>)}</select></label>
        <label><span>收件人</span><input value={draft.to} maxLength={1000} onChange={e => session.update({ to: e.target.value })} placeholder="name@example.com" /></label>
        <label><span>主题</span><input value={draft.subject} maxLength={1000} onChange={e => session.update({ subject: e.target.value })} placeholder="给这封邮件一个主题" /></label>
      </div>
      <textarea className="compose-body" aria-label="邮件正文" value={draft.bodyText} maxLength={50000} onChange={e => session.update({ bodyText: e.target.value })} placeholder="从一句问候开始…" />
      {(state.error || platformError) && <p className="inline-error" role="alert">{state.error || platformError}</p>}
      {sendNotice && <p className="inline-ok" role="status">{sendNotice}</p>}
      {closeTarget && <div className="discard-prompt" role="alert"><span>{state.saving ? '正在保存，请稍候。' : state.dirty ? '还有未保存的修改。' : '修改已保存，可以安全关闭。'}</span><Button onClick={() => void save(true)} disabled={state.saving}>{closeTarget === 'window' ? '保存并退出' : '保存并关闭'}</Button><Button variant="ghost" disabled={state.saving} onClick={() => void finishClose()}>{state.dirty ? '放弃修改' : '关闭'}</Button><Button variant="ghost" onClick={() => { setCloseTarget(null); session.resume(); }}>继续编辑</Button></div>}
      <footer className="compose-footer"><span role="status">{status}</span><Button type="submit" disabled={state.saving}><Save size={15} />{state.saving ? '正在保存…' : '保存草稿'}</Button><Button variant="primary" disabled={!canSend || sending || !!sendNotice} title={sendBlockReason || undefined} onClick={() => void send()}>{sending ? <Loader2 size={15} className="spin" /> : <Send size={15} />}{sending ? '正在登记…' : '发送'}</Button></footer>
    </form>
  </Dialog>;
}
