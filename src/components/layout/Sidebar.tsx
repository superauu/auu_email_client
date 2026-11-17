import { Archive, ChevronDown, ChevronRight, FilePenLine, Folder, Inbox, Pin, PinOff, Plus, Send, Star, Trash2 } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useNavigationStore } from '../../stores/navigationStore';
import { Button } from '../ui/Button';
import { Avatar } from '../ui/Avatar';
import { countForView } from '../../features/mail/viewCounts';
import { accountDirectories, defaultShortcuts, isNavigationActive, navigationKey, type DirectoryEntry, type DirectoryFlags, type NavigationTarget } from '../../domain/navigation';
import type { MailAccount, MailFolder, MailboxKind, ViewCountEntry } from '../../domain/mail';
import { isDesktop } from '../../platform/desktop';
import { useI18n } from '../../i18n/react';

const icons: Record<MailboxKind, typeof Inbox> = { inbox: Inbox, starred: Star, sent: Send, drafts: FilePenLine, archive: Archive, trash: Trash2, other: Folder };
const emptyFolders: MailFolder[] = [];

function NavigationRow({ entry, active, count, onSelect, secondary, pin }: {
  entry: DirectoryEntry; active: boolean; count: ViewCountEntry | null; onSelect: () => void; secondary?: string;
  pin?: { pinned: boolean; onToggle: () => void; accountName: string };
}) {
  const t = useI18n();
  const Icon = icons[entry.kind];
  // 草稿 / 星标显示总数，其余视图显示未读数；计数未加载时隐藏数字（不误报 0）。
  const showTotal = entry.kind === 'drafts' || entry.kind === 'starred';
  const value = count ? (showTotal ? count.total : count.unread) : 0;
  const metric = entry.kind === 'drafts' ? t('sidebar.draftCount', { count: value }) : entry.kind === 'starred' ? t('sidebar.starredCount', { count: value }) : t('sidebar.unreadCount', { count: value });
  const name = secondary ? `${entry.label} · ${secondary}` : entry.label;
  return <div className={`navigation-row${active ? ' is-active' : ''}`}>
    <button type="button" className="navigation-link" aria-label={name} aria-current={active ? 'page' : undefined} title={`${entry.description} · ${metric}`} onClick={onSelect}>
      <Icon size={16} /><span className="navigation-label"><span>{entry.label}</span>{secondary && <small>{secondary}</small>}</span>
      {!entry.available ? <small className="navigation-unavailable">{t('sidebar.unavailable')}</small> : count && value > 0 && <small className="navigation-count" title={metric} aria-hidden>{value}</small>}
    </button>
    {pin && <button type="button" className={`navigation-pin${pin.pinned ? ' is-pinned' : ''}`} aria-label={pin.pinned ? t('sidebar.unpinAria', { account: pin.accountName, folder: entry.label }) : t('sidebar.pinAria', { account: pin.accountName, folder: entry.label })} aria-pressed={pin.pinned} title={pin.pinned ? t('sidebar.unpinTitle') : t('sidebar.pinTitle')} onClick={pin.onToggle}>{pin.pinned ? <PinOff size={13} /> : <Pin size={13} />}</button>}
  </div>;
}

export function Sidebar({ onCompose, onAccounts, onNavigate }: { onCompose: () => void; onAccounts: () => void; onNavigate?: () => void }) {
  const t = useI18n();
  const { accounts, messages, folders, labels, query, viewCounts, setQuery, view } = useWorkspaceStore(useShallow(s => ({ accounts: s.accounts, messages: s.messages, folders: s.folders ?? emptyFolders, labels: s.labels, query: s.query, viewCounts: s.viewCounts, setQuery: s.setQuery, view: s.view })));
  const { pinned, expanded, setExpanded, togglePinned, error } = useNavigationStore(useShallow(s => ({ pinned: s.pinned, expanded: s.expanded, setExpanded: s.setExpanded, togglePinned: s.togglePinned, error: s.error })));
  const openIds = expanded ?? (accounts[0] ? [accounts[0].id] : []);
  // 计数来自 store.viewCounts（桌面端 Rust 聚合，演示端全量计算）；账号目录的可见性旗标同源。
  const resolveCount = (target: NavigationTarget): ViewCountEntry | null => countForView(viewCounts, target);
  const directories = new Map(accounts.map(account => [account.id, accountDirectories(account, folders, directoryFlags(account, messages, viewCounts))]));
  const allEntries = new Map([...directories.values()].flat().map(entry => [navigationKey(entry.target), entry]));
  const pinnedKeys = new Set(pinned.map(navigationKey));
  function navigate(target: NavigationTarget) {
    if (target.accountId) {
      if (!openIds.includes(target.accountId)) setExpanded([...openIds, target.accountId]);
    }
    setQuery({ ...target, folderId: target.folderId ?? null, draftSource: target.draftSource ?? null, labelId: null, search: '' });
    onNavigate?.();
  }
  function accountTree(account: MailAccount) {
    const entries = directories.get(account.id) ?? [];
    const opened = openIds.includes(account.id);
    const inbox = entries.find(entry => entry.kind === 'inbox')!;
    const unread = resolveCount(inbox.target)?.unread ?? 0;
    const controls = `account-folders-${encodeURIComponent(account.id)}`;
    const toggle = () => setExpanded(opened ? openIds.filter(id => id !== account.id) : [...openIds, account.id]);
    return <li className="account-branch" key={account.id}>
      <div className={`account-heading${query.accountId === account.id ? ' is-current' : ''}`}>
        <button className="account-disclosure" type="button" aria-label={opened ? t('sidebar.collapseFolderAria', { account: account.name }) : t('sidebar.expandFolderAria', { account: account.name })} aria-expanded={opened} aria-controls={controls} onClick={toggle}>{opened ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</button>
        <button className="account-select" type="button" aria-label={account.name} aria-pressed={query.accountId === account.id} title={`${account.name} · ${account.email}${account.status === 'demo' ? t('sidebar.demoAccountSuffix') : ''}`} onClick={() => navigate(inbox.target)}>
          <Avatar text={account.avatar} color={account.color} small /><span>{account.name}</span>{account.status === 'demo' && <small className="account-demo-badge" aria-hidden>{t('sidebar.demoBadge')}</small>}{viewCounts && unread > 0 && <small className="navigation-count" aria-hidden title={t('sidebar.inboxUnreadTitle', { unread })}>{unread}</small>}
        </button>
      </div>
      <nav id={controls} className="account-folders" aria-label={t('sidebar.folderNavAria', { account: account.name })} hidden={!opened}>
        {entries.map(entry => <NavigationRow key={navigationKey(entry.target)} entry={entry} active={view === 'mail' && isNavigationActive(query, entry.target)} count={resolveCount(entry.target)} onSelect={() => navigate(entry.target)} pin={{ pinned: pinnedKeys.has(navigationKey(entry.target)), onToggle: () => togglePinned(entry.target), accountName: account.name }} />)}
      </nav>
    </li>;
  }
  return <aside className="sidebar" aria-label={t('sidebar.mailNavAria')}>
    <div className="workspace-name"><span>{t('sidebar.workspaceName')}</span><span className="label">{t('sidebar.workspaceLocal')}</span></div>
    <Button variant="primary" className="compose-button" disabled={!accounts.length} title={accounts.length ? undefined : t('sidebar.composeNeedAccount')} onClick={onCompose}><Plus size={18} />{t('sidebar.compose')}<span>N</span></Button>
    <div className="sidebar-scroll">
      <div className="sidebar-section-title"><span>{t('sidebar.quickSection')}</span><span className="local-hint" title={t('sidebar.quickHintTitle')}>{t('sidebar.quickHint')}</span></div>
      <nav className="shortcut-list" aria-label={t('sidebar.quickNavAria')}>
        {defaultShortcuts().map(entry => <NavigationRow key={navigationKey(entry.target)} entry={entry} active={view === 'mail' && isNavigationActive(query, entry.target)} count={resolveCount(entry.target)} onSelect={() => navigate(entry.target)} />)}
        {pinned.map(target => {
          const account = accounts.find(item => item.id === target.accountId);
          if (!account) return null;
          const entry = allEntries.get(navigationKey(target));
          return entry && account ? <NavigationRow key={navigationKey(target)} entry={entry} secondary={account.name} active={view === 'mail' && isNavigationActive(query, target)} count={resolveCount(target)} onSelect={() => navigate(target)} pin={{ pinned: true, onToggle: () => togglePinned(target), accountName: account.name }} />
            : <div className="navigation-row stale-shortcut" key={navigationKey(target)}><span>{t('sidebar.staleShortcut')}</span><button type="button" className="navigation-pin is-pinned" aria-label={t('sidebar.removeStaleAria')} onClick={() => togglePinned(target)}><PinOff size={14} /></button></div>;
        })}
      </nav>
      <div className="sidebar-divider" />
      <div className="sidebar-section-title"><span>{t('sidebar.accountsSection')}</span><button title={t('sidebar.manageAccounts')} aria-label={t('sidebar.manageAccountsAria')} onClick={onAccounts}><Plus size={14} /></button></div>
      {!accounts.length && <p className="navigation-empty">{t('sidebar.noAccounts')}</p>}
      <ul className="account-tree">{accounts.map(accountTree)}</ul>
      {labels.length > 0 && <><div className="sidebar-divider" /><div className="sidebar-section-title"><span>{t('sidebar.labelsSection')}</span></div><nav className="folder-list" aria-label={t('sidebar.labelsNavAria')}>{labels.map(label => <button key={label.id} title={label.name} className={query.labelId === label.id && view === 'mail' ? 'is-active' : ''} aria-current={query.labelId === label.id && view === 'mail' ? 'page' : undefined} onClick={() => { setQuery({ accountId: label.accountId ?? null, folderId: null, draftSource: null, labelId: label.id, search: '' }); onNavigate?.(); }}><span className={`label-dot tone-${label.color}`} /><span>{label.name}</span></button>)}</nav></>}
      {error && <p className="navigation-error" role="status">{error}</p>}
    </div>
    <div className="sidebar-note"><span className="status-dot" /><div><strong>{t('sidebar.localFirst')}</strong><p>{isDesktop() ? accounts.length ? t('sidebar.mailboxCount', { count: accounts.length }) : t('sidebar.noMailbox') : t('sidebar.demoNote')}</p></div></div>
  </aside>;
}

/** 服务器草稿入口的可见性旗标：演示账号看全量缓存，真实账号看视图计数。 */
function directoryFlags(account: MailAccount, messages: import('../../domain/mail').MailMessage[], viewCounts: ViewCountEntry[] | null): DirectoryFlags {
  if (account.status === 'demo') return { serverDrafts: messages.some(message => message.accountId === account.id && message.mailbox === 'drafts' && !message.draft) };
  return { serverDrafts: (countForView(viewCounts, { accountId: account.id, mailbox: 'drafts' })?.total ?? 0) > 0 };
}
