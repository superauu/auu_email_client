import { ChevronDown, ChevronUp, Archive, ArrowLeft, Check, CornerUpLeft, FolderInput, Forward, Globe, Image, Mail, MailOpen, PenLine, ReplyAll, ShieldCheck, Sparkles, Star, Tag, Trash, Trash2 } from 'lucide-react';
import React from 'react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { useShallow } from 'zustand/react/shallow';
import { replyTargets, type MailMessage, type RecoveryAction } from '../../domain/mail';
import { isPreviewableAttachment } from '../../domain/attachments';
import { linkifyText } from '../../domain/linkify';
import { senderDomain } from '../../domain/trust';
import { errorMessage } from '../../domain/errors';
import { useTrustStore } from '../../stores/trustStore';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useThreadViewStore } from '../../stores/threadViewStore';
import { groupConversations } from './threading';
import { Button } from '../../components/ui/Button';
import { Avatar } from '../../components/ui/Avatar';
import { EmptyState } from '../../components/ui/EmptyState';
import { selectMessages } from './selectors';
import { useSyncStore } from '../../stores/syncStore';
import { desktop, desktopMail, isDesktop } from '../../platform/desktop';
import { useThemeStore } from '../../stores/themeStore';
import { useReaderFontStore } from '../../stores/readerFontStore';
import { readerFontPx as readerFontPxOf } from '../../domain/readerFont';
import { useI18n, currentLanguage } from '../../i18n/react';
import { useRovingTabindex } from '../../hooks/useRovingTabindex';

type HtmlState = { state: 'loading' | 'ready' | 'plain'; doc: string; allowImages: boolean; trustedSender?: boolean };

/** 回复全部：收件目标含 Cc；没有任何可回对象时禁用（等同无意义操作）。 */
function ReplyAllButton({ message, accountEmail, onCompose }: { message: MailMessage; accountEmail: string; onCompose: (message: MailMessage, editing?: boolean, mode?: 'reply' | 'reply-all' | 'forward') => void }) {
  const t = useI18n();
  const targets = replyTargets(message, accountEmail, 'reply-all');
  const disabled = !targets.to.length;
  return (
    <Button variant="secondary" disabled={disabled} title={disabled ? t('reader.replyAllNoOthers') : undefined} onClick={() => onCompose(message, false, 'reply-all')}>
      <ReplyAll size={15} />{t('reader.replyAll')}
    </Button>
  );
}

function PlainBody({ text, empty, onOpen }: { text: string; empty: string; onOpen: (href: string) => void }) {
  const segments = useMemo(() => linkifyText(text), [text]);
  const inDesktop = isDesktop();
  if (!text) return <div className="message-body">{empty}</div>;
  return <div className="message-body">{segments.map((segment, index) => {
    if (segment.kind === 'text') return <span key={index}>{segment.value}</span>;
    // 仅放行 http/https/mailto；tel: 等其余 scheme 保持纯文本。
    if (!/^(https?:\/\/|mailto:)/i.test(segment.href)) return <span key={index}>{segment.value}</span>;
    return inDesktop
      ? <button key={index} type="button" className="mail-link" title={segment.href} onClick={() => onOpen(segment.href)}>{segment.value}</button>
      : <a key={index} className="mail-link" href={segment.href} target="_blank" rel="noreferrer">{segment.value}</a>;
  })}</div>;
}

export function MailReader({ onCompose, onAi, onBack }: { onCompose: (message: MailMessage, editing?: boolean, mode?: 'reply' | 'reply-all' | 'forward') => void; onAi: () => void; onBack: () => void }) {
  const t = useI18n();
  const { messages, selectedId, accounts, labels, query, savingIds, folders, toggleStar, toggleUnread, keepMessageLocal, archiveMessage, deleteMessage, purgeMessage, moveMessage, requestRecovery, messageHtml, setMessageLabels } = useWorkspaceStore(useShallow(s => ({
    messages: s.messages, selectedId: s.selectedId, accounts: s.accounts, labels: s.labels, query: s.query,
    savingIds: s.savingIds, folders: s.folders, toggleStar: s.toggleStar, toggleUnread: s.toggleUnread, keepMessageLocal: s.keepMessageLocal, archiveMessage: s.archiveMessage, deleteMessage: s.deleteMessage, purgeMessage: s.purgeMessage, moveMessage: s.moveMessage, requestRecovery: s.requestRecovery, messageHtml: s.messageHtml, setMessageLabels: s.setMessageLabels,
  })));
  const { enqueue, jobs, submitting, syncError } = useSyncStore(useShallow(s => ({ enqueue: s.enqueue, jobs: s.jobs, submitting: s.submitting, syncError: s.error })));
  const [html, setHtml] = useState<HtmlState>({ state: 'plain', doc: '', allowImages: false });
  const [frameHeight, setFrameHeight] = useState<number | null>(null);
  const [openError, setOpenError] = useState('');
  const [moveOpen, setMoveOpen] = useState(false);
  const openExternalLink = useCallback((href: string) => {
    setOpenError('');
    void desktop.openExternal(href).catch(error => setOpenError(errorMessage(error)));
  }, []);
  // 沙箱 iframe 的唯一通信通道：只接受已知形状的消息，链接再经 Rust scheme 白名单校验。
  useEffect(() => {
    function onMessage(event: MessageEvent) {
      const data = event.data;
      if (!data || typeof data !== 'object') return;
      if (data.type === 'auumail-frame-height' && typeof data.height === 'number' && Number.isFinite(data.height)) {
        setFrameHeight(Math.min(Math.max(220, Math.round(data.height)), 20000));
      }
      if (data.type === 'auumail-mail-link' && typeof data.href === 'string' && data.href.length <= 2048) {
        openExternalLink(data.href);
      }
    }
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [openExternalLink]);
  const message = selectMessages(messages, { ...query, unreadOnly: false }).find(m => m.id === selectedId);
  // 对话模式下的会话内导航：当前邮件所在会话的时间正序成员序列。
  const groupBySubject = useThreadViewStore(state => state.groupBySubject);
  const conversation = useMemo(() => {
    if (!groupBySubject || !message || message.draft) return null;
    return groupConversations(messages.filter(item => item.accountId === message.accountId)).find(group => group.messages.some(item => item.id === message.id)) ?? null;
  }, [groupBySubject, message, messages]);
  const conversationPosition = conversation ? conversation.messages.findIndex(item => item.id === message!.id) : -1;
  const conversationPrev = conversation && conversationPosition > 0 ? conversation.messages[conversationPosition - 1] : null;
  const conversationNext = conversation && conversationPosition >= 0 && conversationPosition + 1 < conversation.messages.length ? conversation.messages[conversationPosition + 1] : null;
  const selectMessage = useWorkspaceStore(state => state.selectMessage);
  const gatewaySupportsPurge = isDesktop();
  const messageId = message?.id ?? null;
  const htmlEligible = !!message && !message.draft && (accounts.find(a => a.id === message.accountId)?.status ?? 'demo') !== 'demo';
  const messageHtmlAction = useWorkspaceStore.getState().messageHtml;
  // 沙箱正文的底色跟随当前主题；切换深浅模式后重新渲染。
  const themeMode = useThemeStore(s => s.resolvedMode);
  // 阅读字号档位（可调整字号）：纯文本经 CSS 变量生效，沙箱正文渲染时传入。
  const readerFontPx = readerFontPxOf(useReaderFontStore(s => s.fontId));
  const roving = useRovingTabindex('horizontal');
  useEffect(() => {
    if (!messageId || !htmlEligible) { setHtml({ state: 'plain', doc: '', allowImages: false }); setFrameHeight(null); return; }
    let active = true;
    setHtml({ state: 'loading', doc: '', allowImages: false });
    setFrameHeight(null);
    async function load() {
      const target = useWorkspaceStore.getState().messages.find(m => m.id === messageId);
      if (!target || target.draft) return;
      try {
        // 信任判定：发件地址或域名在信任列表 → 直接加载远程图片（静默同步同款不打扰语义）。
        await useTrustStore.getState().ensure().catch(() => undefined);
        const trusted = useTrustStore.getState().isTrusted(target.from.email);
        const doc = await messageHtmlAction(target.accountId, target.id, trusted, themeMode, readerFontPx);
        if (!active) return;
        setHtml({ state: 'ready', doc, allowImages: trusted, trustedSender: trusted });
      } catch { if (active) setHtml({ state: 'plain', doc: '', allowImages: false }); }
    }
    void load();
    return () => { active = false; };
  }, [messageId, htmlEligible, messageHtmlAction, themeMode, readerFontPx]);
  if (!message) return <section className="mail-reader"><EmptyState heading="h2" title={t('reader.emptyTitle')} description={t('reader.emptyBody')} /></section>;
  const account = accounts.find(a => a.id === message.accountId);
  const localDraft = Boolean(message.draft);
  const serverDraft = message.mailbox === 'drafts' && !localDraft;
  const saving = savingIds.includes(message.id);
  const changes = message.changes ?? [];
  const remote = changes.filter(change => change.state !== 'local');
  const needsReview = remote.some(change => ['failed', 'conflict', 'unknown'].includes(change.state));
  const active = submitting.includes(message.accountId) || jobs.some(job => job.accountId === message.accountId && ['queued', 'running'].includes(job.status));
  const recovery = message.recovery;
  const checking = recovery?.state === 'checking';
  // 移动族意图 applying / unknown 期间禁止再次移动 / 删除（防重复 MOVE）。
  const moveBlocked = message.changes?.some(change => ['archive', 'delete', 'move'].includes(change.field) && ['applying', 'unknown'].includes(change.state));
  const moveTargets = (folders ?? []).filter(folder => folder.accountId === message.accountId && folder.available && folder.kind !== 'drafts');
  const requestBody = useWorkspaceStore.getState().requestBody;
  async function recover(action: RecoveryAction) {
    if (!message) return;
    if (action === 'check' && message.recovery?.state === 'queued') { await enqueue([message.accountId]); return; }
    if (await requestRecovery(message.id, action)) await enqueue([message.accountId]);
  }
  /** 重新渲染正文（allow=是否放行远程图片）；trust 仅用于横幅状态展示。 */
  async function reloadHtml(allow: boolean, trusted: boolean) {
    if (!message) return;
    const current = message;
    try {
      const doc = await messageHtml(current.accountId, current.id, allow, useThemeStore.getState().resolvedMode, readerFontPxOf(useReaderFontStore.getState().fontId));
      setHtml({ state: 'ready', doc, allowImages: allow, trustedSender: trusted });
      setOpenError('');
    } catch (error) { setHtml({ state: 'plain', doc: '', allowImages: false }); setOpenError(errorMessage(error)); }
  }
  function loadWithImages() { void reloadHtml(true, false); }
  /** 信任发件地址或域名：入库后立即自动加载本封远程图片，此后该来源一律自动加载。 */
  async function trustCurrent(kind: 'sender' | 'domain') {
    if (!message) return;
    const value = kind === 'sender' ? message.from.email.trim().toLowerCase() : senderDomain(message.from.email);
    if (!value) return;
    try { await useTrustStore.getState().add(kind, value); }
    catch (error) { setOpenError(errorMessage(error)); return; }
    await reloadHtml(true, true);
  }
  /** 取消信任：移除该发件地址与其域名的全部条目，恢复拦截。 */
  async function untrustCurrent() {
    if (!message) return;
    const store = useTrustStore.getState();
    try {
      await store.remove('sender', message.from.email.trim().toLowerCase());
      const domain = senderDomain(message.from.email);
      if (domain) await store.remove('domain', domain);
    } catch (error) { setOpenError(errorMessage(error)); return; }
    await reloadHtml(false, false);
  }
  /** 打 / 取消一个用户标签：整体替换该邮件的标签集合（纯本机注解）。 */
  async function toggleTag(labelId: string) {
    if (!message) return;
    const next = message.labelIds.includes(labelId) ? message.labelIds.filter(id => id !== labelId) : [...message.labelIds, labelId];
    await setMessageLabels(message.accountId, message.id, next);
  }
  const richAvailable = html.state === 'ready' && !!html.doc;
  return <section className="mail-reader" aria-label={t('reader.sectionAria')}>
    <div className="reader-toolbar" ref={roving.containerRef as unknown as React.Ref<HTMLDivElement>} onKeyDown={roving.onKeyDown} onFocus={roving.onFocus}><div><Button variant="ghost" iconOnly className="reader-back" aria-label={t('reader.backAria')} onClick={onBack}><ArrowLeft size={17} /></Button><Button variant="ghost" iconOnly title={t('reader.archiveTitle')} aria-label={t('reader.archiveAria')} disabled={saving || message.mailbox === 'archive' || localDraft} onClick={() => archiveMessage(message.id)}><Archive size={17} /></Button><Button variant="ghost" iconOnly disabled={saving || localDraft} aria-label={t('reader.starToggleAria')} aria-pressed={message.starred} onClick={() => toggleStar(message.id)}><Star size={17} fill={message.starred ? 'currentColor' : 'none'} /></Button><Button variant="ghost" iconOnly disabled={saving || localDraft} aria-label={message.unread ? t('reader.markRead') : t('reader.markUnread')} onClick={() => toggleUnread(message.id)}>{message.unread ? <MailOpen size={17} /> : <Mail size={17} />}</Button><Button variant="ghost" iconOnly title={t('reader.deleteTitle')} aria-label={t('reader.deleteAria')} disabled={saving || localDraft || moveBlocked} onClick={() => deleteMessage(message.id)}><Trash2 size={17} /></Button>{message.mailbox === 'trash' && !localDraft && gatewaySupportsPurge && <Button variant="ghost" iconOnly title={t('reader.purge')} aria-label={t('reader.purgeAria')} disabled={saving || moveBlocked} onClick={() => { if (window.confirm(t('reader.purgeConfirm'))) void purgeMessage(message.id); }}><Trash size={17} /></Button>}{moveTargets.length > 0 && <div className="move-menu-wrap"><Button variant="ghost" iconOnly title={t('reader.moveTo')} aria-label={t('reader.moveToAria')} aria-expanded={moveOpen} disabled={saving || localDraft || moveBlocked} onClick={() => setMoveOpen(value => !value)} onKeyDown={event => { if (!moveOpen && (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ')) { event.preventDefault(); setMoveOpen(true); } }}><FolderInput size={17} /></Button>{moveOpen && <div className="move-menu" role="menu" aria-label={t('reader.moveMenuAria')} tabIndex={-1} onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); setMoveOpen(false); } }} onBlur={event => { if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) setMoveOpen(false); }} ref={node => { if (node && !node.contains(document.activeElement)) node.querySelector<HTMLElement>('[role="menuitem"]')?.focus(); }}>{moveTargets.map((folder, index) => <button key={folder.id} role="menuitem" tabIndex={-1} onKeyDown={event => {  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {   event.preventDefault();   const items = Array.from((event.currentTarget.parentNode as HTMLElement).querySelectorAll<HTMLElement>('[role="menuitem"]'));   items[(index + (event.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus();  }  if (event.key === 'Escape') { event.stopPropagation(); setMoveOpen(false); } }} onClick={() => { setMoveOpen(false); moveMessage(message.id, folder.id); }}>{t('reader.moveTargetAria', { name: folder.name })}</button>)}</div>}</div>}{isDesktop() && !localDraft && labels.length > 0 && <DropdownMenu.Root>
                    <DropdownMenu.Trigger asChild><Button variant="ghost" iconOnly aria-label={t('reader.labelMenuAria')} title={t('reader.labelMenuAria')} disabled={saving}><Tag size={17} /></Button></DropdownMenu.Trigger>
                    <DropdownMenu.Portal>
                      <DropdownMenu.Content side="bottom" align="start" className="avatar-menu">
                        {labels.map(label => <DropdownMenu.CheckboxItem key={label.id} className="avatar-menu-item" checked={message.labelIds.includes(label.id)} onCheckedChange={() => void toggleTag(label.id)}>
                          <DropdownMenu.ItemIndicator forceMount className="avatar-menu-check"><Check size={14} /></DropdownMenu.ItemIndicator>
                          <span className={`label-dot tone-${label.color}`} aria-hidden /><span>{label.name}</span>
                        </DropdownMenu.CheckboxItem>)}
                      </DropdownMenu.Content>
                    </DropdownMenu.Portal>
                  </DropdownMenu.Root>}</div>{conversation && <><Button variant="ghost" iconOnly title={t('reader.conversationPrevTitle')} aria-label={t('reader.conversationPrevAria')} disabled={!conversationPrev} onClick={() => conversationPrev && selectMessage(conversationPrev.id)}><ChevronUp size={16} /></Button><Button variant="ghost" iconOnly title={t('reader.conversationNextTitle')} aria-label={t('reader.conversationNextAria')} disabled={!conversationNext} onClick={() => conversationNext && selectMessage(conversationNext.id)}><ChevronDown size={16} /></Button><span className="conversation-position" title={t('reader.conversationPositionTitle', { position: conversationPosition + 1, total: conversation.messages.length })}>{t('reader.conversationPosition', { position: conversationPosition + 1, total: conversation.messages.length })}</span></>}<Button variant="ghost" onClick={onAi}><Sparkles size={16} /> {t('reader.ai')}</Button></div>
    <div className="reader-scroll"><div className="reader-content">
      {(saving || changes.length > 0 || recovery) && <div className="message-change-status" role="status" aria-live="polite">
        <span>{saving ? t('reader.savingLocal') : remote.length ? needsReview ? t('reader.savedNeedsReview') : t('reader.savedPendingWriteback') : t('reader.savedLocal')}</span>
        {remote.map(change => change.error).filter((error, index, all) => error && all.indexOf(error) === index).map(error => <p key={error}>{error}</p>)}
        {remote.length > 0 && <><Button variant="ghost" disabled={saving || active} onClick={() => void enqueue([message.accountId])}>{active ? t('reader.accountSyncing') : t('reader.syncAccount')}</Button>{needsReview && <Button variant="ghost" disabled={saving || remote.some(change => change.state === 'applying')} onClick={() => keepMessageLocal(message.id)}>{t('reader.keepLocal')}</Button>}</>}
        {syncError && remote.length > 0 && <p>{syncError}</p>}
        {recovery && <div>
          <p>{recovery.detail}</p>
          {recovery.progress && (recovery.progress.segments ? <p>{t('reader.segmentsProgress', { phase: recovery.progress.segments.phase === 'collecting' ? t('reader.phaseCollecting') : t('reader.phaseValidating'), completed: recovery.progress.segments.completed, total: recovery.progress.segments.total, checked: recovery.progress.checked, matches: recovery.progress.matches })}</p> : <p>{t('reader.progress', { checked: recovery.progress.checked, total: recovery.progress.total, matches: recovery.progress.matches })}</p>)}
          {recovery.fingerprint && <><p>{recovery.fingerprint.reason}</p><Button variant="ghost" disabled={!recovery.fingerprint.eligible || saving || active || checking || recovery.state === 'queued'} onClick={() => void recover('fingerprint')}>{t('reader.fingerprintAction')}</Button></>}
          {recovery.crossUidValidity && <p>{t('reader.crossUidValidity')}</p>}
          {recovery.location && <p>{t('reader.checkLocation', { folder: recovery.location.folder, uidValidity: recovery.location.uidValidity, uid: recovery.location.uid })}</p>}
          {recovery.state !== 'restored' && <Button variant="ghost" disabled={saving || active || checking} onClick={() => void recover('check')}>{active || checking ? t('reader.recoveryBusy') : recovery.state === 'queued' ? t('reader.startQueuedCheck') : t('reader.checkRemote')}</Button>}
          {recovery.state === 'located' && <Button variant="ghost" disabled={saving || active} onClick={() => void recover(recovery.operation === 'fingerprint' ? 'confirm-fingerprint' : 'confirm')}>{recovery.operation === 'fingerprint' ? t('reader.confirmFingerprint') : t('reader.confirmRestore')}</Button>}
          {recovery.state === 'source-present' && <Button variant="ghost" disabled={saving || active} onClick={() => void recover('retry-archive')}>{t('reader.requeueArchive')}</Button>}
          {(recovery.state === 'scanning' || (recovery.state === 'failed' && recovery.progress)) && <Button variant="ghost" disabled={saving || active} onClick={() => void recover('continue')}>{recovery.progress?.segments ? recovery.progress.segments.phase === 'validating' ? t('reader.continueValidating') : t('reader.continueSegments') : t('reader.continueNextPage')}</Button>}
          {(recovery.state === 'scanning' || (recovery.state === 'failed' && recovery.progress)) && <Button variant="ghost" disabled={saving || checking} onClick={() => void recover('auto-continue')}>{t('reader.autoContinuePages')}</Button>}
          {recovery.autoContinue && recovery.state === 'queued' && <p>{t('reader.autoContinueNote')}</p>}
          {!remote.length && recovery.state !== 'restored' && recovery.state !== 'local' && <Button variant="ghost" disabled={saving} onClick={() => keepMessageLocal(message.id)}>{t('reader.keepLocal')}</Button>}
          {syncError && !remote.length && <p>{syncError}</p>}
        </div>}
      </div>}
      <div className="reader-labels"><span className={`label tone-${account?.color ?? 'teal'}`}>{account?.name}</span>{message.labelIds.map(id => { const label = labels.find(l => l.id === id); return label && <span key={id} className={`label tone-${label.color}`}>{label.name}</span>; })}{localDraft && <span className="label">{t('reader.localDraftLabel')}</span>}{serverDraft && <span className="label">{t('reader.serverDraftCopyLabel')}</span>}</div>
      <h2>{message.subject}</h2>
      <div className="message-sender"><Avatar text={message.from.name.slice(0, 1)} color={account?.color} /><div><strong>{message.from.name}</strong><span>{message.from.email}</span><span>{t('reader.to', { value: message.to.join(', ') || t('reader.toEmpty') })}</span>{!!message.cc?.length && <span>{t('reader.cc', { value: message.cc.join(', ') })}</span>}</div><time dateTime={message.receivedAt}>{new Intl.DateTimeFormat(currentLanguage(), { month: 'long', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(new Date(message.receivedAt))}</time></div>
      <div className="reader-actions" role="toolbar" aria-label={t('reader.toolbarAria')}>
        <Button variant="primary" onClick={() => onCompose(message, message.mailbox === 'drafts')}>{localDraft ? <PenLine size={15} /> : <CornerUpLeft size={15} />}{localDraft ? t('reader.editDraft') : serverDraft ? t('reader.copyToLocal') : t('reader.reply')}</Button>
        {!localDraft && !serverDraft && <>
          <ReplyAllButton message={message} accountEmail={account?.email ?? ''} onCompose={onCompose} />
          <Button variant="secondary" onClick={() => onCompose(message, false, 'forward')}><Forward size={15} />{t('reader.forward')}</Button>
        </>}
        <span>{localDraft ? t('reader.localDraftNote') : serverDraft ? t('reader.serverDraftNote') : t('reader.replyFrom', { email: account?.email ?? '' })}</span>
      </div>
      {(account?.status !== 'demo' && !localDraft || message.remoteMissing) && <div className="message-meta">
        {account?.status !== 'demo' && !localDraft && <Button variant="ghost" disabled={saving || active || checking || message.remoteMissing} onClick={() => void (async () => {
          if (message.bodyDownload?.state === 'queued' || await requestBody(message.id)) await enqueue([message.accountId]);
        })()}>{active && message.bodyDownload?.state === 'queued' ? t('reader.downloadingBody') : message.bodyDownload?.state === 'queued' ? t('reader.continueBodyDownload') : t('reader.redownloadBody')}</Button>}
        {message.bodyDownload?.error && <p role="status">{message.bodyDownload.error}</p>}
        {message.remoteMissing && <p className="muted" role="status">{t('reader.remoteMissingNote')}</p>}
      </div>}
      {!!message.attachments?.length && <AttachmentList key={message.id} message={message} />}
      {richAvailable
        ? <div className="mail-html-wrapper">
          {html.state === 'ready' && html.allowImages && html.trustedSender ? <div className="html-remote-note" role="status"><span>{t('reader.trustedNote')}</span><Button variant="ghost" onClick={() => void untrustCurrent()}>{t('reader.untrust')}</Button></div>
            : html.allowImages ? null
            : <div className="html-remote-note" role="status"><span>{t('reader.remoteImagesBlocked')}</span>
                <span className="remote-actions">
                  <Button variant="ghost" onClick={loadWithImages}><Image size={14} />{t('reader.loadRemoteImages')}</Button>
                  {isDesktop() && !localDraft && <DropdownMenu.Root>
                    <DropdownMenu.Trigger asChild><Button variant="ghost" iconOnly aria-label={t('reader.trustMenuAria')} title={t('reader.trustMenuAria')}><ChevronDown size={14} /></Button></DropdownMenu.Trigger>
                    <DropdownMenu.Portal>
                      <DropdownMenu.Content side="bottom" align="start" className="avatar-menu">
                        <DropdownMenu.Item className="avatar-menu-item" onSelect={() => void trustCurrent('sender')}><ShieldCheck size={15} /><span>{t('reader.trustSender')}</span></DropdownMenu.Item>
                        <DropdownMenu.Item className="avatar-menu-item" onSelect={() => void trustCurrent('domain')}><Globe size={15} /><span>{t('reader.trustDomain')}</span></DropdownMenu.Item>
                      </DropdownMenu.Content>
                    </DropdownMenu.Portal>
                  </DropdownMenu.Root>}
                </span>
              </div>}
          <iframe className="mail-html-frame" sandbox="allow-scripts" srcDoc={html.doc} title={t('reader.htmlFrameTitle')} style={frameHeight ? { height: `${frameHeight}px` } : undefined} />
        </div>
        : <PlainBody text={message.bodyText} empty={localDraft ? t('reader.emptyLocalDraft') : t('reader.emptyPlainBody')} onOpen={openExternalLink} />}
      {openError && <p className="inline-error" role="alert">{openError}</p>}
    </div></div>
  </section>;
}
function AttachmentList({ message }: { message: MailMessage }) {
  const t = useI18n();
  const exportAttachment = useWorkspaceStore(state => state.exportAttachment);
  const [busy, setBusy] = useState<number | null>(null);
  const [saved, setSaved] = useState('');
  /** 内联预览：每附件独立展开；图片为 Rust 白名单 data URI，文本为转义安全的纯文本。 */
  const [previews, setPreviews] = useState<Record<number, import('../../domain/attachments').AttachmentPreview | null>>({});
  const [previewError, setPreviewError] = useState('');
  async function save(index: number, name: string) {
    setBusy(index); setSaved('');
    if (await exportAttachment(message.id, index)) setSaved(t('reader.savedAttachment', { name }));
    setBusy(null);
  }
  async function togglePreview(attachment: { index: number }) {
    const existing = previews[attachment.index];
    setPreviewError('');
    if (existing) { setPreviews(current => ({ ...current, [attachment.index]: null })); return; }
    setBusy(attachment.index);
    try {
      const preview = await desktopMail.previewAttachment(message.accountId, message.id, attachment.index);
      setPreviews(current => ({ ...current, [attachment.index]: preview }));
    } catch (error) { setPreviewError(errorMessage(error)); }
    setBusy(null);
  }
  return <section aria-label={t('reader.attachmentsAria')} className="message-change-status">
    <strong>{t('reader.attachments')}</strong>
    {message.attachments?.map(attachment => <div key={attachment.index}>
      <span>{attachment.name} · {new Intl.NumberFormat(currentLanguage(), { maximumFractionDigits: 1 }).format(attachment.size / 1024)} KB</span>
      {isDesktop() && !message.draft && isPreviewableAttachment(attachment.mediaType, attachment.name) && <Button variant="ghost" disabled={busy !== null} aria-expanded={Boolean(previews[attachment.index])} aria-label={previews[attachment.index] ? t('reader.previewHideAria', { name: attachment.name }) : t('reader.previewAria', { name: attachment.name })} onClick={() => void togglePreview(attachment)}>{previews[attachment.index] ? t('reader.previewHide') : busy === attachment.index ? t('reader.previewLoading') : t('reader.preview')}</Button>}
      <Button variant="ghost" disabled={busy !== null} aria-label={t('reader.saveAttachmentAria', { name: attachment.name })} onClick={() => void save(attachment.index, attachment.name)}>{busy === attachment.index ? t('reader.saving') : t('reader.saveAs')}</Button>
      {previews[attachment.index]?.kind === 'image' && <img className="attachment-preview" src={previews[attachment.index]!.dataUri ?? ''} alt={attachment.name} />}
      {previews[attachment.index]?.kind === 'text' && <pre className="attachment-preview-text">{previews[attachment.index]!.text}</pre>}
      {previews[attachment.index]?.kind === 'pdf' && <iframe className="attachment-preview-pdf" src={previews[attachment.index]!.dataUri ?? ''} title={attachment.name} />}
      {previews[attachment.index]?.truncated && <span className="muted" role="status">{t('reader.previewTruncated')}</span>}
    </div>)}
    {previewError && <p className="inline-error" role="alert">{previewError}</p>}
    <span role="status" aria-live="polite">{saved}</span>
  </section>;
}
