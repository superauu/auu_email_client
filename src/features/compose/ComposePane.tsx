import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { ArrowLeft, Check, Loader2, Save, Send, Maximize2, Minimize2, Paperclip, Type, Trash2 } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import type { MailDraft } from '../../domain/mail';
import { errorMessage } from '../../domain/errors';
import { desktop, isDesktop } from '../../platform/desktop';
import { DraftSession } from './DraftSession';
import { RichEditor, escapeHtml } from './RichEditor';
import { RecipientField } from './RecipientField';
import { isRecipientAddress, recipientError, splitRecipients } from '../../domain/recipients';

export function ComposePane({ initial, onClose, onNotice }: { initial: MailDraft; onClose: () => void; onNotice?: (text: string) => void }) {
  const accounts = useWorkspaceStore(s => s.accounts);
  const messages = useWorkspaceStore(s => s.messages);
  const saveDraft = useWorkspaceStore(s => s.saveDraft);
  const sendDraft = useWorkspaceStore(s => s.sendDraft);
  const messageHtmlFragment = useWorkspaceStore(s => s.messageHtmlFragment);
  const deleteDraft = useWorkspaceStore(s => s.deleteDraft);
  const [session] = useState(() => new DraftSession(initial, saveDraft));
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { draft } = state;
  const [closeTarget, setCloseTarget] = useState<'pane' | 'window' | null>(null);
  const [platformError, setPlatformError] = useState('');
  const [sendNotice, setSendNotice] = useState('');
  const [sending, setSending] = useState(false);
  const [discarding, setDiscarding] = useState(false);
  const [quoteHtml, setQuoteHtml] = useState<string | null>(null);
  const [quoteLoading, setQuoteLoading] = useState(!initial.bodyHtml && !initial.revision && !!initial.replyToId);
  const [showCc, setShowCc] = useState(!!initial.cc);
  const [showBcc, setShowBcc] = useState(!!initial.bcc);
  const [expanded, setExpanded] = useState(false);
  const [formatting, setFormatting] = useState(true);
  const [closing, setClosing] = useState(false);
  const [confirmSend, setConfirmSend] = useState(false);
  const busy = sending || discarding || closing;
  const suggestions = useMemo(() => {
    const contacts = new Map<string, { name: string; email: string }>();
    for (const message of messages.filter(item => item.accountId === draft.accountId && !item.draft)) {
      for (const contact of [message.from, ...message.to.map(email => ({ name: '', email }))]) {
        if (isRecipientAddress(contact.email) && !contacts.has(contact.email.toLowerCase())) contacts.set(contact.email.toLowerCase(), contact);
      }
    }
    return [...contacts.values()];
  }, [messages, draft.accountId]);
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

  // 回复 / 转发引用：取消毒后的原文片段（无缓存时保留既有纯文本引用）。
  const quoteInjected = useRef(false);
  useEffect(() => {
    if (quoteInjected.current || initial.bodyHtml || initial.revision || !initial.replyToId) return;
    quoteInjected.current = true;
    void (async () => {
      const origin = useWorkspaceStore.getState().messages.find(message => message.id === initial.replyToId);
      try {
        const fragment = await messageHtmlFragment(initial.accountId, initial.replyToId!);
        const header = origin
          ? `---------- 原始邮件 ----------<br>发件人：${escapeHtml(origin.from.name)} &lt;${escapeHtml(origin.from.email)}&gt;<br>主题：${escapeHtml(origin.subject)}`
          : '---------- 原始邮件 ----------';
        setQuoteHtml(`<p><br></p><div style="color:var(--text-muted);font-size:12px;border-top:1px solid var(--border);padding-top:8px">${header}</div><blockquote data-auumail-quote="true" style="margin:0 0 0 12px;border-left:2px solid var(--border);padding-left:12px">${fragment}</blockquote>`);
      } catch {
        // 无已验证原文：回退纯文本引用。
        if (origin?.bodyText) {
          const escaped = escapeHtml(origin.bodyText).replace(/\n/g, '<br>');
          const header = `---------- 原始邮件 ----------<br>发件人：${escapeHtml(origin.from.name)} &lt;${escapeHtml(origin.from.email)}&gt;`;
          setQuoteHtml(`<p><br></p><div style="color:var(--text-muted);font-size:12px;border-top:1px solid var(--border);padding-top:8px">${header}</div><blockquote data-auumail-quote="true" style="margin:0 0 0 12px;border-left:2px solid var(--border);padding-left:12px">${escaped}</blockquote>`);
        } else if (initial.bodyText) setQuoteHtml(`<blockquote data-auumail-quote="true">${escapeHtml(initial.bodyText).replace(/\n/g, '<br>')}</blockquote>`);
      } finally { setQuoteLoading(false); }
    })();
  }, [initial, messageHtmlFragment]);

  useEffect(() => {
    function beforeUnload(event: BeforeUnloadEvent) {
      if (!allowWindowClose.current && session.needsProtection()) { event.preventDefault(); event.returnValue = ''; }
    }
    window.addEventListener('beforeunload', beforeUnload);
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // 窗口关闭：先自动保存草稿再放行；保存失败才弹出兜底确认条。
    void desktop.guardWindowClose(() => !allowWindowClose.current && session.needsProtection(), () => {
      session.pause();
      setClosing(true);
      void (async () => {
        const saved = await session.flush();
        if (saved) {
          allowWindowClose.current = true;
          try { await desktop.windowAction('close'); }
          catch (error) { allowWindowClose.current = false; session.resume(); setPlatformError(errorMessage(error)); }
        } else setCloseTarget('window');
        setClosing(false);
      })();
    }).then(cleanup => { if (disposed) cleanup(); else unlisten = cleanup; }).catch(error => {
      if (!disposed) setPlatformError(`未能启用窗口关闭保护：${errorMessage(error)}`);
    });
    return () => { disposed = true; unlisten?.(); session.cancelScheduledSave(); window.removeEventListener('beforeunload', beforeUnload); };
  }, [session]);

  // 关闭撰写面：自动保存草稿（无需手动点击）；保存失败才出现兜底选择条。
  async function requestClose() {
    if (busy) return;
    if (!session.needsProtection()) { onClose(); return; }
    setClosing(true);
    setPlatformError('');
    session.pause();
    if (await session.flush()) onClose();
    else setCloseTarget('pane');
    setClosing(false);
  }
  async function finishClose() {
    session.cancelScheduledSave();
    if (closeTarget === 'window') {
      allowWindowClose.current = true;
      try { await desktop.windowAction('close'); }
      catch (error) { allowWindowClose.current = false; setPlatformError(errorMessage(error)); }
    } else onClose();
  }
  /** 显式放弃：删除本机草稿并关闭撰写面（这是唯一的删除入口）。 */
  async function discard() {
    setPlatformError('');
    setDiscarding(true);
    session.pause();
    try {
      await session.waitForSave();
      await deleteDraft(draft.id);
      onClose();
    } catch (error) {
      setPlatformError(`草稿未能删除：${errorMessage(error)}`);
      if (!closeTarget) session.resume();
    }
    setDiscarding(false);
  }
  async function save(closeAfter?: boolean) {
    setPlatformError('');
    if (await session.flush()) {
      if (closeAfter) await finishClose();
    }
  }
  async function send() {
    const invalid = recipientError([draft.to, draft.cc ?? '', draft.bcc ?? '']);
    if (invalid) { setPlatformError(invalid); return; }
    if (!draft.subject.trim()) { setConfirmSend(true); return; }
    await performSend();
  }
  async function performSend() {
    if (!canSend || busy || quoteLoading) return;
    const current = session.getSnapshot().draft;
    const invalid = recipientError([current.to, current.cc ?? '', current.bcc ?? '']);
    if (invalid) { setPlatformError(invalid); return; }
    session.update({ to: splitRecipients(current.to).join(', '), cc: splitRecipients(current.cc ?? '').join(', '), bcc: splitRecipients(current.bcc ?? '').join(', ') });
    setPlatformError('');
    setSending(true);
    setConfirmSend(false);
    try {
      // 先保证草稿落盘并拿到当前 revision，再登记发送队列（docs/SENDING.md）。
      if (!(await session.flush())) throw new Error('草稿尚未保存成功，请稍后重试。');
      const entry = await sendDraft(draft.id, session.currentRevision);
      const notice = entry.state === 'queued' ? '已进入发送队列；发送结果可在设置 → 邮箱账号查看，结果不明时不会自动重发。' : `发送任务状态：${entry.state}`;
      setSendNotice(notice);
      setCloseTarget(null);
      session.resume();
      // 登记成功即离开撰写面；结果以 toast 与账号状态徽标反馈。
      onNotice?.(notice);
      onClose();
    } catch (error) { setPlatformError(errorMessage(error)); }
    finally { setSending(false); }
  }
  // Escape 关闭：与返回按钮一致，先自动保存草稿。
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.defaultPrevented || event.isComposing || document.querySelector('[role="dialog"]')) return;
      if (event.key === 'Escape') { event.preventDefault(); void requestClose(); }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); if (!busy) void save(); }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
  const status = state.error ? '保存失败，内容仍保留在窗口中' : state.saving ? '正在保存草稿…' : state.dirty ? '正在自动保存草稿…' : state.saved ? '草稿已自动保存' : '修改后将自动保存草稿';
  const title = initial.replyToId ? /^转发[:：]/.test(initial.subject) ? '转发邮件' : '回复邮件' : '新邮件';
  return <section className={`compose-pane${expanded ? ' is-expanded' : ''}`} aria-label={title}>
    <div className="reader-toolbar"><Button variant="ghost" iconOnly aria-label="关闭撰写并返回列表" disabled={busy} onClick={() => void requestClose()}><ArrowLeft size={17} /></Button><h2>{title}</h2><div className="compose-pane-head-actions"><Button variant="ghost" aria-label="保存草稿" title="保存草稿 (Ctrl+S)" onClick={() => void save()} disabled={state.saving || busy}><Save size={15} /><span>保存草稿</span></Button><Button variant="ghost" iconOnly aria-label={expanded ? '还原撰写窗格' : '展开撰写窗格'} aria-pressed={expanded} onClick={() => setExpanded(value => !value)}>{expanded ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</Button></div></div>
    <div className="compose-commandbar">
      <Button variant="primary" disabled={!canSend || busy || quoteLoading || !!sendNotice} title={sendBlockReason || (quoteLoading ? '正在准备引用原文' : '发送邮件')} onClick={() => void send()}>{sending ? <Loader2 size={15} className="spin" /> : <Send size={15} />}{sending ? '正在登记…' : '发送'}</Button>
      <Button variant="ghost" aria-pressed={formatting} onClick={() => setFormatting(value => !value)}><Type size={16} />格式</Button>
      <span title="附件发送尚未接通"><Button variant="ghost" disabled aria-label="添加附件（尚未支持）"><Paperclip size={16} />附件</Button></span>
      <span className="compose-local-label">本机草稿</span>
    </div>
    <div className="compose-pane-body">
      <div className="compose-fields">
        <div className="compose-address-row"><label htmlFor="compose-from">发件人</label><select id="compose-from" value={draft.accountId} onChange={e => session.update({ accountId: e.target.value })} disabled={!!draft.replyToId || busy} title={draft.replyToId ? '回复草稿使用原邮件所属账号' : undefined}>{accounts.map(a => <option key={a.id} value={a.id}>{a.name} &lt;{a.email}&gt;</option>)}</select></div>
        <RecipientField label="收件人" value={draft.to} disabled={busy} autoFocus={!initial.replyToId && !initial.revision} suggestions={suggestions} onChange={to => session.update({ to })}><div className="compose-recipient-toggles"><button type="button" aria-expanded={showCc} disabled={!!draft.cc || busy} onClick={() => setShowCc(value => !value)}>抄送</button><button type="button" aria-expanded={showBcc} disabled={!!draft.bcc || busy} onClick={() => setShowBcc(value => !value)}>密送</button></div></RecipientField>
        {showCc && <RecipientField label="抄送" value={draft.cc ?? ''} disabled={busy} suggestions={suggestions} onChange={cc => session.update({ cc })} />}
        {showBcc && <RecipientField label="密送" value={draft.bcc ?? ''} disabled={busy} suggestions={suggestions} onChange={bcc => session.update({ bcc })} />}
        <div className="compose-address-row"><label htmlFor="compose-subject">主题</label><input id="compose-subject" value={draft.subject} disabled={busy} maxLength={1000} onChange={e => { session.update({ subject: e.target.value }); setConfirmSend(false); }} placeholder="添加主题" /></div>
      </div>
      {quoteLoading && <p className="compose-quote-loading" role="status">正在准备引用原文…</p>}
      <RichEditor initialHtml={initial.bodyHtml ?? ''} initialText={initial.replyToId && !initial.revision && !initial.bodyHtml ? '' : initial.bodyText} quoteHtml={quoteHtml} disabled={busy} formatting={formatting} onChange={(html, text) => session.update({ bodyHtml: html, bodyText: text })} />
    </div>
    {(state.error || platformError) && <p className="compose-feedback inline-error" role="alert">{state.error || platformError}</p>}
    {confirmSend && <div className="discard-prompt" role="alert"><span>这封邮件还没有主题，仍然发送吗？</span><Button disabled={busy} onClick={() => void performSend()}>仍然发送</Button><Button variant="ghost" onClick={() => { setConfirmSend(false); document.getElementById('compose-subject')?.focus(); }}>填写主题</Button></div>}
    {closeTarget && <div className="discard-prompt" role="alert"><span>自动保存失败，内容仍保留在本窗口中。</span><Button onClick={() => void save(true)} disabled={state.saving || busy}>{closeTarget === 'window' ? '保存并退出' : '保存并关闭'}</Button><Button variant="ghost" disabled={state.saving || busy} onClick={() => void discard()}>放弃草稿</Button><Button variant="ghost" onClick={() => { setCloseTarget(null); session.resume(); }}>继续编辑</Button></div>}
    <footer className="compose-pane-footer"><span role="status">{state.saving ? <Loader2 size={13} className="spin" /> : state.saved && !state.dirty ? <Check size={13} /> : <Save size={13} />}{status}</span><Button variant="ghost" className="button-danger-ghost" disabled={state.saving || busy} title="删除这份草稿并关闭撰写面" onClick={() => void discard()}><Trash2 size={15} />{discarding ? '正在删除…' : '放弃草稿'}</Button></footer>
    <div className="compose-capability">{canSend ? `发件账号：${account?.email}` : sendBlockReason} · 暂不支持附件与图片发送</div>
  </section>;
}
