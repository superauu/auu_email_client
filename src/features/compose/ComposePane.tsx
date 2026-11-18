import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { ArrowLeft, Check, Loader2, Save, Send, Maximize2, Minimize2, Paperclip, Type, Trash2, X } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import type { MailDraft } from '../../domain/mail';
import { errorMessage } from '../../domain/errors';
import { desktop, isDesktop } from '../../platform/desktop';
import { DraftSession } from './DraftSession';
import { RichEditor, escapeHtml } from './RichEditor';
import { useSignatureStore } from '../../stores/signatureStore';
import { RecipientField } from './RecipientField';
import { isRecipientAddress, recipientError, splitRecipients } from '../../domain/recipients';
import { useI18n, currentLanguage } from '../../i18n/react';

/** 与 MailReader 一致的 KB 尺寸展示。 */
function attachmentSize(size: number): string {
  return `${new Intl.NumberFormat(currentLanguage(), { maximumFractionDigits: 1 }).format(size / 1024)} KB`;
}

export function ComposePane({ initial, onClose, onNotice }: { initial: MailDraft; onClose: () => void; onNotice?: (text: string) => void }) {
  const t = useI18n();
  const accounts = useWorkspaceStore(s => s.accounts);
  const messages = useWorkspaceStore(s => s.messages);
  const saveDraft = useWorkspaceStore(s => s.saveDraft);
  const sendDraft = useWorkspaceStore(s => s.sendDraft);
  const messageHtmlFragment = useWorkspaceStore(s => s.messageHtmlFragment);
  const deleteDraft = useWorkspaceStore(s => s.deleteDraft);
  const pickAttachments = useWorkspaceStore(s => s.pickAttachments);
  const [session] = useState(() => new DraftSession(initial, saveDraft));
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { draft } = state;
  const [closeTarget, setCloseTarget] = useState<'pane' | 'window' | null>(null);
  const [platformError, setPlatformError] = useState('');
  const [sendNotice, setSendNotice] = useState('');
  const [sending, setSending] = useState(false);
  const [discarding, setDiscarding] = useState(false);
  const [picking, setPicking] = useState(false);
  const [quoteHtml, setQuoteHtml] = useState<string | null>(null);
  const [quoteLoading, setQuoteLoading] = useState(!initial.bodyHtml && !initial.revision && !!initial.replyToId);
  const [showCc, setShowCc] = useState(!!initial.cc);
  const [showBcc, setShowBcc] = useState(!!initial.bcc);
  const [expanded, setExpanded] = useState(false);
  const [formatting, setFormatting] = useState(true);
  const [closing, setClosing] = useState(false);
  const [confirmSend, setConfirmSend] = useState(false);
  const busy = sending || discarding || closing;
  const attachments = draft.attachments ?? [];
  // 收件人建议：桌面端由 Rust 聚合最近邮件（大邮箱不再要求全量消息进内存）；失败回退到已缓存消息。
  const contactSuggestions = useWorkspaceStore(s => s.contactSuggestions);
  const [remoteContacts, setRemoteContacts] = useState<{ name: string; email: string }[] | null>(null);
  useEffect(() => {
    let alive = true;
    void contactSuggestions(draft.accountId)
      .then(list => { if (alive) setRemoteContacts(list); })
      .catch(() => { if (alive) setRemoteContacts(null); });
    return () => { alive = false; };
  }, [contactSuggestions, draft.accountId]);
  const suggestions = useMemo(() => {
    if (remoteContacts) return remoteContacts;
    const contacts = new Map<string, { name: string; email: string }>();
    for (const message of messages.filter(item => item.accountId === draft.accountId && !item.draft)) {
      for (const contact of [message.from, ...message.to.map(email => ({ name: '', email })), ...(message.cc ?? message.replyTo ?? []).map(email => ({ name: '', email }))]) {
        if (isRecipientAddress(contact.email) && !contacts.has(contact.email.toLowerCase())) contacts.set(contact.email.toLowerCase(), contact);
      }
    }
    return [...contacts.values()];
  }, [remoteContacts, messages, draft.accountId]);
  const allowWindowClose = useRef(false);
  const account = accounts.find(a => a.id === draft.accountId);
  const canSend = isDesktop() && account?.capabilities.send === true;
  const sendBlockReason = !isDesktop()
    ? t('compose.sendBlockedBrowser')
    : !account || account.status === 'demo'
      ? t('compose.sendBlockedDemo')
      : account.capabilities.send
        ? ''
        : t('compose.sendBlockedCapability');

  // 回复 / 转发引用：取消毒后的原文片段（无缓存时保留既有纯文本引用）。
  // 签名（新邮件按 useNew、回复 / 转发按 useReply）与引用并行取；签名先落位再注入引用，
  // 保证签名位于引用块上方（Outlook 语义）。已保存草稿（有 revision / 正文）不重复追加。
  const quoteInjected = useRef(false);
  const [signatureHtml, setSignatureHtml] = useState<string | null>(null);
  useEffect(() => {
    if (quoteInjected.current || initial.bodyHtml || initial.revision) return;
    quoteInjected.current = true;
    void (async () => {
      // 签名读取失败静默置空（撰写不被阻塞），演示端无签名。
      let signature = '';
      if (isDesktop()) {
        try {
          const value = await useSignatureStore.getState().load(initial.accountId);
          if (initial.replyToId ? value.useReply : value.useNew) signature = value.html;
        } catch { /* 无签名 */ }
      }
      setSignatureHtml(signature || null);
      if (!initial.replyToId) return;
      const origin = useWorkspaceStore.getState().messages.find(message => message.id === initial.replyToId);
      try {
        const fragment = await messageHtmlFragment(initial.accountId, initial.replyToId!);
        const header = origin
          ? [t('compose.quoteHeaderPlain'), t('compose.quoteFrom', { value: `${escapeHtml(origin.from.name)} &lt;${escapeHtml(origin.from.email)}&gt;` }), t('compose.quoteSubject', { value: escapeHtml(origin.subject) })].join('<br>')
          : t('compose.quoteHeaderPlain');
        setQuoteHtml(`<p><br></p><div style="color:var(--text-muted);font-size:12px;border-top:1px solid var(--border);padding-top:8px">${header}</div><blockquote data-auumail-quote="true" style="margin:0 0 0 12px;border-left:2px solid var(--border);padding-left:12px">${fragment}</blockquote>`);
      } catch {
        // 无已验证原文：回退纯文本引用。
        if (origin?.bodyText) {
          const escaped = escapeHtml(origin.bodyText).replace(/\n/g, '<br>');
          const header = [t('compose.quoteHeaderPlain'), t('compose.quoteFrom', { value: `${escapeHtml(origin.from.name)} &lt;${escapeHtml(origin.from.email)}&gt;` })].join('<br>');
          setQuoteHtml(`<p><br></p><div style="color:var(--text-muted);font-size:12px;border-top:1px solid var(--border);padding-top:8px">${header}</div><blockquote data-auumail-quote="true" style="margin:0 0 0 12px;border-left:2px solid var(--border);padding-left:12px">${escaped}</blockquote>`);
        } else if (initial.bodyText) setQuoteHtml(`<blockquote data-auumail-quote="true">${escapeHtml(initial.bodyText).replace(/\n/g, '<br>')}</blockquote>`);
      } finally { setQuoteLoading(false); }
    })();
  }, [initial, messageHtmlFragment, t]);

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
      if (!disposed) setPlatformError(t('compose.windowGuardFailed', { error: errorMessage(error) }));
    });
    return () => { disposed = true; unlisten?.(); session.cancelScheduledSave(); window.removeEventListener('beforeunload', beforeUnload); };
  }, [session, t]);

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
      setPlatformError(t('compose.deleteFailed', { error: errorMessage(error) }));
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
  /** 附件选取：Rust 侧对话框 + 暂存，返回新增元数据；取消为空数组（无操作）。 */
  async function attach() {
    if (busy || picking) return;
    setPlatformError('');
    setPicking(true);
    try {
      const current = session.getSnapshot().draft.attachments ?? [];
      const picked = await pickAttachments(draft.id, current);
      if (picked.length) session.update({ attachments: [...current, ...picked] });
    } catch (error) {
      setPlatformError(errorMessage(error));
    } finally {
      setPicking(false);
    }
  }
  function removeAttachment(id: string) {
    session.update({ attachments: (session.getSnapshot().draft.attachments ?? []).filter(item => item.id !== id) });
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
      if (!(await session.flush())) throw new Error(t('compose.notSavedYet'));
      const entry = await sendDraft(draft.id, session.currentRevision);
      const notice = entry.state === 'queued' ? t('compose.queuedNotice') : t('compose.sendTaskState', { state: entry.state });
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
  const status = state.error ? t('compose.statusSaveFailed') : state.saving ? t('compose.statusSaving') : state.dirty ? t('compose.statusAutoSaving') : state.saved ? t('compose.statusSaved') : t('compose.statusIdle');
  const title = initial.replyToId ? /^转发[:：]/.test(initial.subject) ? t('compose.titleForward') : t('compose.titleReply') : t('compose.titleNew');
  return <section className={`compose-pane${expanded ? ' is-expanded' : ''}`} aria-label={title}>
    <div className="reader-toolbar"><Button variant="ghost" iconOnly aria-label={t('compose.closeAria')} disabled={busy} onClick={() => void requestClose()}><ArrowLeft size={17} /></Button><h2>{title}</h2><div className="compose-pane-head-actions"><Button variant="ghost" aria-label={t('compose.saveDraft')} title={t('compose.saveDraftTitle')} onClick={() => void save()} disabled={state.saving || busy}><Save size={15} /><span>{t('compose.saveDraft')}</span></Button><Button variant="ghost" iconOnly aria-label={expanded ? t('compose.restoreAria') : t('compose.expandAria')} aria-pressed={expanded} onClick={() => setExpanded(value => !value)}>{expanded ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</Button></div></div>
    <div className="compose-commandbar">
      <Button variant="primary" disabled={!canSend || busy || quoteLoading || !!sendNotice} title={sendBlockReason || (quoteLoading ? t('compose.sendTitleQuoteLoading') : t('compose.sendTitle'))} onClick={() => void send()}>{sending ? <Loader2 size={15} className="spin" /> : <Send size={15} />}{sending ? t('compose.sending') : t('compose.send')}</Button>
      <Button variant="ghost" aria-pressed={formatting} onClick={() => setFormatting(value => !value)}><Type size={16} />{t('compose.format')}</Button>
      {isDesktop()
        ? <Button variant="ghost" disabled={busy || picking || attachments.length >= 20} aria-label={t('compose.attachAria')} title={t('compose.attachTitle')} onClick={() => void attach()}>{picking ? <Loader2 size={16} className="spin" /> : <Paperclip size={16} />}{picking ? t('compose.picking') : t('compose.attach')}</Button>
        : <span title={t('compose.attachBrowserTitle')}><Button variant="ghost" disabled aria-label={t('compose.attachBrowserAria')}><Paperclip size={16} />{t('compose.attach')}</Button></span>}
      <span className="compose-local-label">{t('compose.localDraftChip')}</span>
    </div>
    <div className="compose-pane-body">
      <div className="compose-fields">
        <div className="compose-address-row"><label htmlFor="compose-from">{t('compose.fromLabel')}</label><select id="compose-from" value={draft.accountId} onChange={e => session.update({ accountId: e.target.value })} disabled={!!draft.replyToId || busy} title={draft.replyToId ? t('compose.fromReplyLockedTitle') : undefined}>{accounts.map(a => <option key={a.id} value={a.id}>{a.name} &lt;{a.email}&gt;</option>)}</select></div>
        <RecipientField label={t('compose.toLabel')} value={draft.to} disabled={busy} autoFocus={!initial.replyToId && !initial.revision} suggestions={suggestions} onChange={to => session.update({ to })}><div className="compose-recipient-toggles"><button type="button" aria-expanded={showCc} disabled={!!draft.cc || busy} onClick={() => setShowCc(value => !value)}>{t('compose.ccLabel')}</button><button type="button" aria-expanded={showBcc} disabled={!!draft.bcc || busy} onClick={() => setShowBcc(value => !value)}>{t('compose.bccLabel')}</button></div></RecipientField>
        {showCc && <RecipientField label={t('compose.ccLabel')} value={draft.cc ?? ''} disabled={busy} suggestions={suggestions} onChange={cc => session.update({ cc })} />}
        {showBcc && <RecipientField label={t('compose.bccLabel')} value={draft.bcc ?? ''} disabled={busy} suggestions={suggestions} onChange={bcc => session.update({ bcc })} />}
        <div className="compose-address-row"><label htmlFor="compose-subject">{t('compose.subjectLabel')}</label><input id="compose-subject" value={draft.subject} disabled={busy} maxLength={1000} onChange={e => { session.update({ subject: e.target.value }); setConfirmSend(false); }} placeholder={t('compose.subjectPlaceholder')} /></div>
      </div>
      {attachments.length > 0 && <ul className="compose-attachments" aria-label={t('compose.attachmentsListAria')}>
        {attachments.map(item => <li key={item.id} className="compose-attachment">
          <Paperclip size={13} aria-hidden />
          <span className="compose-attachment-name" title={item.name}>{item.name}</span>
          <span className="compose-attachment-size">{attachmentSize(item.size)}</span>
          <Button variant="ghost" iconOnly aria-label={t('compose.removeAttachmentAria', { name: item.name })} title={t('compose.removeAttachmentTitle')} disabled={busy || picking} onClick={() => removeAttachment(item.id)}><X size={14} /></Button>
        </li>)}
      </ul>}
      {quoteLoading && <p className="compose-quote-loading" role="status">{t('compose.quoteLoading')}</p>}
      <RichEditor initialHtml={initial.bodyHtml ?? ''} initialText={initial.replyToId && !initial.revision && !initial.bodyHtml ? '' : initial.bodyText} quoteHtml={quoteHtml} signatureHtml={signatureHtml} disabled={busy} formatting={formatting} onChange={(html, text) => session.update({ bodyHtml: html, bodyText: text })} />
    </div>
    {(state.error || platformError) && <p className="compose-feedback inline-error" role="alert">{state.error || platformError}</p>}
    {confirmSend && <div className="discard-prompt" role="alert"><span>{t('compose.emptySubjectConfirm')}</span><Button disabled={busy} onClick={() => void performSend()}>{t('compose.sendAnyway')}</Button><Button variant="ghost" onClick={() => { setConfirmSend(false); document.getElementById('compose-subject')?.focus(); }}>{t('compose.fillSubject')}</Button></div>}
    {closeTarget && <div className="discard-prompt" role="alert"><span>{t('compose.autosaveFailed')}</span><Button onClick={() => void save(true)} disabled={state.saving || busy}>{closeTarget === 'window' ? t('compose.saveAndExit') : t('compose.saveAndClose')}</Button><Button variant="ghost" disabled={state.saving || busy} onClick={() => void discard()}>{t('compose.discardDraft')}</Button><Button variant="ghost" onClick={() => { setCloseTarget(null); session.resume(); }}>{t('compose.keepEditing')}</Button></div>}
    <footer className="compose-pane-footer"><span role="status">{state.saving ? <Loader2 size={13} className="spin" /> : state.saved && !state.dirty ? <Check size={13} /> : <Save size={13} />}{status}</span><Button variant="ghost" className="button-danger-ghost" disabled={state.saving || busy} title={t('compose.discardTitle')} onClick={() => void discard()}><Trash2 size={15} />{discarding ? t('compose.deleting') : t('compose.discardDraft')}</Button></footer>
    <div className="compose-capability">{canSend ? t('compose.capabilityFrom', { email: account?.email ?? '' }) : sendBlockReason}{isDesktop() ? t('compose.imagePasteNote') : t('compose.attachDesktopLimitNote')}</div>
  </section>;
}
