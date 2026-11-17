import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDown, Check, FolderInput, ListChecks, RefreshCw, Star, X } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useSyncStore } from '../../stores/syncStore';
import { useThemeStore } from '../../stores/themeStore';
import { mailboxName, selectMessages } from './selectors';
import { Button } from '../../components/ui/Button';
import { EmptyState } from '../../components/ui/EmptyState';
import { cn } from '../../lib/cn';
import { isDesktop } from '../../platform/desktop';
import { isSyncActive, syncableAccounts } from '../../domain/sync';
import { directoryLabel } from '../../domain/navigation';
import { useI18n, currentLanguage } from '../../i18n/react';
import type { MailMessage, MessagePatch } from '../../domain/mail';
import { virtualWindow } from './virtual';

/** 读取当前皮肤的列表行高 token；参数仅作为切换时的缓存失效标记。 */
function readRowHeight(_skinId: string): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue('--list-row-height').trim();
  const value = parseFloat(raw);
  return Number.isFinite(value) && value > 0 ? value : 76;
}

export function MailList({ onSelect }: { onSelect: () => void }) {
  const t = useI18n();
  const { messages, accounts, labels, query, selectedId, savingIds, folders, selectMessage, toggleStar, setQuery, applyMessagePatch } = useWorkspaceStore(useShallow(s => ({
    messages: s.messages, accounts: s.accounts, labels: s.labels, query: s.query, selectedId: s.selectedId,
    savingIds: s.savingIds, folders: s.folders, selectMessage: s.selectMessage, toggleStar: s.toggleStar, setQuery: s.setQuery,
    applyMessagePatch: s.applyMessagePatch,
  })));
  const { jobs, submitting, enqueue } = useSyncStore(useShallow(s => ({ jobs: s.jobs, submitting: s.submitting, enqueue: s.enqueue })));
  const [selected, setSelected] = useState<string[]>([]);
  const [batchRunning, setBatchRunning] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  // ── 虚拟列表：只渲染视口窗口，滚动条高度由上下留白维持 ──
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const skinId = useThemeStore(state => state.skinId);
  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    let frame = 0;
    const update = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => { frame = 0; setScrollTop(element.scrollTop); });
    };
    const measure = () => { setViewportHeight(element.clientHeight); setScrollTop(element.scrollTop); };
    measure();
    element.addEventListener('scroll', update, { passive: true });
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => { element.removeEventListener('scroll', update); observer.disconnect(); if (frame) cancelAnimationFrame(frame); };
  }, []);
  // 行高取皮肤 token（Paper 76 / Graphite 72 / Linen 80 等），随 skin 切换生效。
  const rowHeight = useMemo(() => readRowHeight(skinId), [skinId]);
  const jobsByAccount = new Map(jobs.map(job => [job.accountId, job]));
  const visible = selectMessages(messages, query);
  const window_ = virtualWindow({ count: visible.length, rowHeight, scrollTop, viewportHeight });
  // 视图切换 / 搜索变化时清空选择，避免跨视图误操作。
  useEffect(() => { setSelected([]); setMoveOpen(false); }, [query.accountId, query.mailbox, query.folderId, query.draftSource, query.labelId, query.search, query.unreadOnly]);
  const queriedAccount = query.accountId ? accounts.find(a => a.id === query.accountId) : null;
  const realAccount = queriedAccount && queriedAccount.status !== 'demo' ? queriedAccount : null;
  // 指定账号视图同步该账号；全部账号视图同步所有可同步账号（IMAP + Gmail OAuth）。
  const syncTarget = realAccount;
  const syncable = syncableAccounts(accounts);
  const syncingAll = syncable.some(({ id }) => submitting.includes(id) || jobs.some(job => job.accountId === id && isSyncActive(job)));
  const accountSyncing = syncTarget ? submitting.includes(syncTarget.id) || (jobsByAccount.get(syncTarget.id) ? isSyncActive(jobsByAccount.get(syncTarget.id)!) : false) : false;
  function syncCurrent() {
    if (!isDesktop()) return;
    const ids = syncTarget ? [syncTarget.id] : syncable.map(({ id }) => id);
    if (!ids.length || (syncTarget ? accountSyncing : syncingAll)) return;
    void enqueue(ids);
  }
  const title = query.labelId ? labels.find(l => l.id === query.labelId)?.name : query.draftSource === 'local' ? t('nav.allLocalDrafts') : query.draftSource === 'server' ? t('nav.mailbox.serverDrafts') : !query.accountId && query.mailbox === 'inbox' ? t('nav.allInbox') : mailboxName(query.mailbox);
  const folder = (folders ?? []).find(item => item.id === query.folderId && item.accountId === query.accountId);
  const folderTitle = folder ? directoryLabel(folder, (folders ?? []).filter(item => item.accountId === query.accountId)) : undefined;
  // ── 多选批量操作 ──────────────────────────────────────────
  const selectionMode = selected.length > 0;
  const selectedAccounts = new Set(selected.map(id => messages.find(m => m.id === id)?.accountId).filter(Boolean) as string[]);
  const batchMoveAllowed = selectedAccounts.size === 1;
  const batchTargets = batchMoveAllowed && isDesktop()
    ? (folders ?? []).filter(f => f.accountId === [...selectedAccounts][0] && f.available && f.kind !== 'drafts')
    : [];
  function toggleSelected(id: string) {
    setSelected(current => current.includes(id) ? current.filter(value => value !== id) : [...current, id]);
  }
  async function runBatch(patchFor: (message: MailMessage) => MessagePatch | null) {
    if (batchRunning || !selected.length) return;
    setBatchRunning(true);
    try {
      for (const id of [...selected]) {
        const message = useWorkspaceStore.getState().messages.find(item => item.id === id);
        if (!message || message.draft) continue;
        const patch = patchFor(message);
        if (!patch) continue;
        await applyMessagePatch(id, patch);
      }
    } finally {
      setBatchRunning(false);
      setSelected([]);
      setMoveOpen(false);
    }
  }
  return <section className="mail-list" aria-label={t('mailList.listAria')}>
    <h1 className="sr-only">{folderTitle ?? title} {visible.length}</h1>
    {selectionMode ? <header className="list-toolbar batch-bar">
      <span className="batch-count">{t('mailList.selectedCount', { count: selected.length })}</span>
      <div className="list-actions batch-actions">
        <Button variant="ghost" disabled={batchRunning} onClick={() => void runBatch(() => ({ unread: false }))}>{t('mailList.batchRead')}</Button>
        <Button variant="ghost" disabled={batchRunning} onClick={() => void runBatch(() => ({ unread: true }))}>{t('mailList.batchUnread')}</Button>
        <Button variant="ghost" disabled={batchRunning} onClick={() => void runBatch(message => message.starred ? null : { starred: true })}>{t('mailList.batchStar')}</Button>
        <Button variant="ghost" disabled={batchRunning} onClick={() => void runBatch(message => message.mailbox === 'archive' ? null : { archive: true })}>{t('mailList.batchArchive')}</Button>
        <Button variant="ghost" disabled={batchRunning} onClick={() => void runBatch(() => ({ delete: true }))}>{t('mailList.batchDelete')}</Button>
        {batchTargets.length > 0 && <div className="move-menu-wrap">
          <Button variant="ghost" disabled={batchRunning} aria-expanded={moveOpen} aria-label={t('mailList.batchMoveAria')} onClick={() => setMoveOpen(value => !value)}><FolderInput size={14} />{t('mailList.batchMove')}</Button>
          {moveOpen && <div className="move-menu" role="menu" aria-label={t('mailList.batchMoveMenuAria')}>
            {batchTargets.map(folder => <button key={folder.id} role="menuitem" onClick={() => { setMoveOpen(false); void runBatch(() => ({ moveTo: folder.id })); }}>{t('mailList.batchMoveTargetAria', { name: folder.name })}</button>)}
          </div>}
        </div>}
        {batchRunning && <span role="status">{t('mailList.batchRunning')}</span>}
        <Button variant="ghost" aria-label={t('mailList.batchCancel')} disabled={batchRunning} onClick={() => setSelected([])}><X size={14} />{t('mailList.batchCancel')}</Button>
      </div>
    </header> : <header className="list-toolbar">
      <div className="list-filters" role="group" aria-label={t('mailList.filterGroupAria')}>
        <button aria-pressed={!query.unreadOnly} onClick={() => setQuery({ unreadOnly: false })}>{t('mailList.filterAll')}</button>
        <button aria-pressed={Boolean(query.unreadOnly)} onClick={() => setQuery({ unreadOnly: true })}>{t('mailList.filterUnread')}</button>
      </div>
      <div className="list-actions"><span className="list-order" title={t('mailList.orderByDateTitle')}>{t('mailList.orderByDate')}<ArrowDown size={12} /></span><Button variant="ghost" aria-label={t('mailList.selectAllAria')} title={t('mailList.selectAll')} disabled={!visible.length} onClick={() => setSelected(visible.map(message => message.id))}><ListChecks size={14} /></Button><Button variant="ghost" className="account-sync" aria-label={syncTarget ? t('mailList.syncAccountAria', { name: syncTarget.name }) : t('mailList.syncAllAria')} title={syncTarget ? t('mailList.syncAccountAria', { name: syncTarget.name }) : t('mailList.syncAllAria')} disabled={!isDesktop() || !syncable.length || (syncTarget ? accountSyncing : syncingAll)} onClick={syncCurrent}><RefreshCw size={14} className={(syncTarget ? accountSyncing : syncingAll) ? 'spin' : ''} />{syncTarget ? submitting.includes(syncTarget.id) ? t('settings.queuing') : jobsByAccount.get(syncTarget.id)?.status === 'queued' ? t('settings.queued') : jobsByAccount.get(syncTarget.id)?.status === 'running' ? t('settings.syncing') : jobsByAccount.get(syncTarget.id)?.status === 'failed' ? t('settings.retry') : t('settings.sync') : syncingAll ? t('settings.syncing') : t('settings.sync')}</Button></div>
    </header>}
    <div className="mail-list-scroll" ref={scrollRef}>
      {visible.length === 0 ? (query.unreadOnly ? <EmptyState title={t('mailList.emptyUnreadTitle')} description={t('mailList.emptyUnreadBody')} /> : query.search ? <EmptyState title={t('mailList.emptySearchTitle')} description={t('mailList.emptySearchBody')} /> : realAccount ? <EmptyState title={t('mailList.emptyAccountTitle', { name: realAccount.name })} description={t('mailList.emptyAccountBody')} /> : <EmptyState title={t('mailList.emptyTitle')} description={t('mailList.emptyBody')} />) : <>
        {window_.topPad > 0 && <div style={{ height: window_.topPad }} aria-hidden />}
        {visible.slice(window_.start, window_.end).map((message, offset) => {
        const account = accounts.find(a => a.id === message.accountId);
        const isSelected = selected.includes(message.id);
        const position = window_.start + offset + 1;
        return <article key={message.id} aria-posinset={position} aria-setsize={visible.length} className={cn('mail-row', selectedId === message.id && 'is-selected', message.unread && 'is-unread', isSelected && 'is-checked')}>
          <button className={cn('row-select', isSelected && 'is-on')} aria-label={t('mailList.selectAria', { subject: message.subject })} aria-pressed={isSelected} onClick={() => toggleSelected(message.id)}><Check size={13} /></button>
          <button className="mail-row-main" title={[message.from.email, account?.name, ...message.labelIds.map(id => labels.find(label => label.id === id)?.name)].filter(Boolean).join(' · ')} aria-label={t('mailList.readAria', { subject: message.subject })} onClick={() => { if (!selectionMode) { selectMessage(message.id); onSelect(); } else toggleSelected(message.id); }} aria-current={selectedId === message.id ? 'true' : undefined}>
            <div className="mail-row-top"><span className="sender"><span className={cn('unread-dot', !message.unread && 'invisible')} /><span>{message.from.name}</span></span><time dateTime={message.receivedAt}>{new Intl.DateTimeFormat(currentLanguage(), { month: 'numeric', day: 'numeric' }).format(new Date(message.receivedAt))}</time></div>
            <div className="mail-row-subject">{message.subject}</div><p>{message.preview || t('mailList.noBodyPreview')}</p>
          </button>
          {!query.accountId && <span className={`row-account tone-${account?.color ?? 'teal'}`} title={account?.email}>{account?.name}</span>}
          <button className={cn('row-star', message.starred && 'is-starred')} aria-label={message.starred ? t('mailList.starRemoveAria', { subject: message.subject }) : t('mailList.starAddAria', { subject: message.subject })} disabled={savingIds.includes(message.id) || Boolean(message.draft)} aria-pressed={message.starred} onClick={() => toggleStar(message.id)}><Star size={14} fill={message.starred ? 'currentColor' : 'none'} /></button>
        </article>;
      })}
        {window_.bottomPad > 0 && <div style={{ height: window_.bottomPad }} aria-hidden />}
      </>}
    </div>
    <footer className="list-footer">{t('mailList.footerCount', { count: visible.length })}<span>{isDesktop() ? t('mailList.footerCache') : t('mailList.footerDemo')}</span></footer>
  </section>;
}
