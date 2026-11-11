import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { CalendarDays, CircleHelp, Mail, Minus, Palette, PanelRight, Search, Settings, Square, X } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { Sidebar } from '../components/layout/Sidebar';
import { Button } from '../components/ui/Button';
import { EmptyState } from '../components/ui/EmptyState';
import { ResizeHandle } from '../components/ui/ResizeHandle';
import { MailList } from '../features/mail/MailList';
import { MailReader } from '../features/mail/MailReader';
import { ComposeDialog } from '../features/compose/ComposeDialog';
import { SettingsDialog } from '../features/settings/SettingsDialog';
import { AiPanel } from '../features/ai/AiPanel';
import { EventsView } from '../features/events/EventsView';
import { useWorkspaceStore } from '../stores/workspaceStore';
import { useSyncStore } from '../stores/syncStore';
import { useLayoutStore, MIN_LIST_WIDTH, MAX_LIST_WIDTH, DEFAULT_LIST_WIDTH, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, DEFAULT_SIDEBAR_WIDTH } from '../stores/layoutStore';
import { desktop, isDesktop } from '../platform/desktop';
import { errorMessage } from '../domain/errors';
import type { MailDraft, MailMessage } from '../domain/mail';

export function App() {
  const { status, error, load, query, setQuery, accounts, view, setView } = useWorkspaceStore(useShallow(s => ({ status: s.status, error: s.error, load: s.load, query: s.query, setQuery: s.setQuery, accounts: s.accounts, view: s.view, setView: s.setView })));
  const { listWidth, sidebarWidth, setListWidth, setSidebarWidth } = useLayoutStore(useShallow(s => ({ listWidth: s.listWidth, sidebarWidth: s.sidebarWidth, setListWidth: s.setListWidth, setSidebarWidth: s.setSidebarWidth })));
  const [settingsTab, setSettingsTab] = useState<'appearance' | 'accounts' | 'about' | null>(null);
  const [draft, setDraft] = useState<MailDraft | null>(null);
  const [aiOpen, setAiOpen] = useState(false);
  const [mobileReading, setMobileReading] = useState(false);
  const [notice, setNotice] = useState('');
  const search = useRef<HTMLInputElement>(null);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => useSyncStore.getState().monitor(), []);

  function compose(message?: MailMessage, editing = false) {
    const accountId = message?.accountId ?? query.accountId ?? accounts[0]?.id;
    if (!accountId) return;
    if (editing && message?.draft) { setDraft({ ...message.draft }); return; }
    setDraft({
      id: `draft-${crypto.randomUUID()}`, accountId,
      to: editing ? message?.to.join(', ') ?? '' : message?.from.email ?? '',
      subject: editing ? message?.subject ?? '' : message ? /^re:/i.test(message.subject) ? message.subject : `Re: ${message.subject}` : '',
      bodyText: editing ? message?.bodyText ?? '' : '',
      ...(!editing && message ? { replyToId: message.id } : {}), updatedAt: new Date().toISOString(),
    });
  }
  const composeRef = useRef(compose);
  useEffect(() => { composeRef.current = compose; });
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') { if (!draft && !settingsTab) { event.preventDefault(); search.current?.focus(); } return; }
      if (event.key.toLowerCase() === 'n' && !event.ctrlKey && !event.metaKey && !event.altKey && !draft && !settingsTab && !(target instanceof HTMLElement && (target.closest('input, textarea, select, [contenteditable="true"]')))) { event.preventDefault(); composeRef.current(); }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [draft, settingsTab]);
  useEffect(() => {
    if (!notice) return;
    const timeout = window.setTimeout(() => setNotice(''), 6000);
    return () => clearTimeout(timeout);
  }, [notice]);

  function windowAction(action: 'minimize' | 'maximize' | 'close' | 'drag') { void desktop.windowAction(action).catch(e => setNotice(errorMessage(e))); }
  return <div className={`app-shell ${mobileReading ? 'mobile-reading' : ''}`} style={{ '--mail-list-width': `${listWidth}px`, '--sidebar-width': `${sidebarWidth}px` } as CSSProperties}>
    <header className="app-header"><div className="brand" onPointerDown={event => { if (isDesktop() && event.button === 0) windowAction('drag'); }}><img src="/logo.svg" alt="" draggable={false} /><strong>AuuMail</strong><span className="brand-divider" /><span className="brand-subtitle">让邮件井然有序</span></div><label className="global-search"><Search size={16} /><input ref={search} aria-label="搜索邮件" type="search" placeholder="搜索邮件、联系人或关键词" value={query.search} onChange={event => { setMobileReading(false); setQuery({ search: event.target.value }); }} /><kbd>Ctrl K</kbd></label><div className="header-actions"><span className="demo-badge">{accounts.some(account => account.status !== 'demo') ? '开发预览' : '演示预览'}</span><Button variant="ghost" iconOnly aria-label="外观与皮肤" title="外观与皮肤" onClick={() => setSettingsTab('appearance')}><Palette size={18} /></Button><Button variant="ghost" iconOnly aria-label="设置" title="设置" onClick={() => setSettingsTab('accounts')}><Settings size={18} /></Button>{isDesktop() && <div className="window-controls"><button aria-label="最小化" onClick={() => windowAction('minimize')}><Minus size={15} /></button><button aria-label="最大化或还原" onClick={() => windowAction('maximize')}><Square size={12} /></button><button className="window-close" aria-label="关闭窗口" onClick={() => windowAction('close')}><X size={17} /></button></div>}</div></header>
    <div className="app-workspace"><nav className="activity-rail" aria-label="主导航"><button className={view === 'mail' ? 'is-active' : ''} aria-label="邮件工作区" aria-current={view === 'mail' ? 'page' : undefined} onClick={() => setView('mail')}><Mail size={21} /></button><button className={view === 'events' ? 'is-active' : ''} aria-label="事件与待办" aria-current={view === 'events' ? 'page' : undefined} onClick={() => setView('events')}><CalendarDays size={21} /></button><div className="rail-spacer" /><button aria-label="关于 AuuMail" onClick={() => setSettingsTab('about')}><CircleHelp size={20} /></button><span className="profile-avatar">A</span></nav>
      <Sidebar onCompose={() => compose()} onAccounts={() => setSettingsTab('accounts')} />
      <ResizeHandle variant="sidebar" label="调整导航栏宽度" value={sidebarWidth} min={MIN_SIDEBAR_WIDTH} max={MAX_SIDEBAR_WIDTH} onResize={setSidebarWidth} onReset={() => setSidebarWidth(DEFAULT_SIDEBAR_WIDTH)} />
      {status === 'error' ? <EmptyState title="工作区未能加载" description={error ?? '请重试。'} action={<Button onClick={() => void load()}>重新加载</Button>} /> : status !== 'ready' ? <div className="workspace-loading" role="status">正在准备工作区…</div> : view === 'events' ? <EventsView /> : <main className="mail-area"><MailList onSelect={() => setMobileReading(true)} /><ResizeHandle label="调整邮件列表宽度" value={listWidth} min={MIN_LIST_WIDTH} max={MAX_LIST_WIDTH} onResize={setListWidth} onReset={() => setListWidth(DEFAULT_LIST_WIDTH)} /><MailReader onCompose={compose} onAi={() => setAiOpen(value => !value)} onBack={() => setMobileReading(false)} />{aiOpen && <AiPanel onClose={() => setAiOpen(false)} />}</main>}
    </div><footer className="app-statusbar"><span><span className="status-dot" />{accounts.some(account => account.status !== 'demo') ? '本地工作区' : '本机演示'} · {accounts.length} 个邮箱账号</span><span><PanelRight size={12} />{isDesktop() ? '桌面工作区' : '浏览器预览'}<span className="status-version">AuuMail 0.1.0</span></span></footer>
    {settingsTab && <SettingsDialog key={settingsTab} open initialTab={settingsTab} onClose={() => setSettingsTab(null)} />}
    {draft && <ComposeDialog key={draft.id} initial={draft} onClose={() => setDraft(null)} />}
    {error && status === 'ready' ? <div className="toast" role="alert">{error}<Button variant="ghost" onClick={() => useWorkspaceStore.getState().clearError()}>关闭提示</Button></div> : notice && <div className="toast" role="status">{notice}</div>}
  </div>;
}
