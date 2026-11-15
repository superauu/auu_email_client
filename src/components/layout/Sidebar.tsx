import { Archive, ChevronDown, ChevronRight, FilePenLine, Folder, Inbox, Pin, PinOff, Plus, Send, Star, Trash2 } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useNavigationStore } from '../../stores/navigationStore';
import { Button } from '../ui/Button';
import { Avatar } from '../ui/Avatar';
import { selectMessages } from '../../features/mail/selectors';
import { accountDirectories, defaultShortcuts, isNavigationActive, navigationKey, type DirectoryEntry, type NavigationTarget } from '../../domain/navigation';
import type { MailAccount, MailFolder, MailMessage, MailboxKind } from '../../domain/mail';
import { isDesktop } from '../../platform/desktop';

const icons: Record<MailboxKind, typeof Inbox> = { inbox: Inbox, starred: Star, sent: Send, drafts: FilePenLine, archive: Archive, trash: Trash2, other: Folder };
const emptyFolders: MailFolder[] = [];

function NavigationRow({ entry, active, messages, onSelect, secondary, pin }: {
  entry: DirectoryEntry; active: boolean; messages: MailMessage[]; onSelect: () => void; secondary?: string;
  pin?: { pinned: boolean; onToggle: () => void; accountName: string };
}) {
  const Icon = icons[entry.kind];
  const items = selectMessages(messages, { ...entry.target, labelId: null, search: '' });
  const total = entry.kind === 'drafts' || entry.kind === 'starred';
  const count = total ? items.length : items.filter(message => message.unread).length;
  const metric = entry.kind === 'drafts' ? `${count} 份草稿` : entry.kind === 'starred' ? `${count} 封星标邮件` : `${count} 封未读`;
  const name = secondary ? `${entry.label} · ${secondary}` : entry.label;
  return <div className={`navigation-row${active ? ' is-active' : ''}`}>
    <button type="button" className="navigation-link" aria-label={name} aria-current={active ? 'page' : undefined} title={`${entry.description} · ${metric}`} onClick={onSelect}>
      <Icon size={16} /><span className="navigation-label"><span>{entry.label}</span>{secondary && <small>{secondary}</small>}</span>
      {!entry.available ? <small className="navigation-unavailable">不可用</small> : count > 0 && <small className="navigation-count" title={metric} aria-hidden>{count}</small>}
    </button>
    {pin && <button type="button" className={`navigation-pin${pin.pinned ? ' is-pinned' : ''}`} aria-label={`${pin.pinned ? '取消固定' : '固定'} ${pin.accountName} / ${entry.label}`} aria-pressed={pin.pinned} title={pin.pinned ? '从常用移除' : '固定到常用'} onClick={pin.onToggle}>{pin.pinned ? <PinOff size={13} /> : <Pin size={13} />}</button>}
  </div>;
}

export function Sidebar({ onCompose, onAccounts, onNavigate }: { onCompose: () => void; onAccounts: () => void; onNavigate?: () => void }) {
  const { accounts, messages, folders, labels, query, setQuery, view } = useWorkspaceStore(useShallow(s => ({ accounts: s.accounts, messages: s.messages, folders: s.folders ?? emptyFolders, labels: s.labels, query: s.query, setQuery: s.setQuery, view: s.view })));
  const { pinned, expanded, setExpanded, togglePinned, error } = useNavigationStore(useShallow(s => ({ pinned: s.pinned, expanded: s.expanded, setExpanded: s.setExpanded, togglePinned: s.togglePinned, error: s.error })));
  const openIds = expanded ?? (accounts[0] ? [accounts[0].id] : []);
  const directories = new Map(accounts.map(account => [account.id, accountDirectories(account, folders, messages)]));
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
    const unread = selectMessages(messages, { ...inbox.target, labelId: null, search: '', unreadOnly: true }).length;
    const controls = `account-folders-${encodeURIComponent(account.id)}`;
    const toggle = () => setExpanded(opened ? openIds.filter(id => id !== account.id) : [...openIds, account.id]);
    return <li className="account-branch" key={account.id}>
      <div className={`account-heading${query.accountId === account.id ? ' is-current' : ''}`}>
        <button className="account-disclosure" type="button" aria-label={`${opened ? '折叠' : '展开'}${account.name}文件夹`} aria-expanded={opened} aria-controls={controls} onClick={toggle}>{opened ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</button>
        <button className="account-select" type="button" aria-label={account.name} aria-pressed={query.accountId === account.id} title={`${account.name} · ${account.email}${account.status === 'demo' ? ' · 演示账号' : ''}`} onClick={() => navigate(inbox.target)}>
          <Avatar text={account.avatar} color={account.color} small /><span>{account.name}</span>{account.status === 'demo' && <small className="account-demo-badge" aria-hidden>演示</small>}{unread > 0 && <small className="navigation-count" aria-hidden title={`${unread} 封收件箱未读`}>{unread}</small>}
        </button>
      </div>
      <nav id={controls} className="account-folders" aria-label={`${account.name}的文件夹`} hidden={!opened}>
        {entries.map(entry => <NavigationRow key={navigationKey(entry.target)} entry={entry} active={view === 'mail' && isNavigationActive(query, entry.target)} messages={messages} onSelect={() => navigate(entry.target)} pin={{ pinned: pinnedKeys.has(navigationKey(entry.target)), onToggle: () => togglePinned(entry.target), accountName: account.name }} />)}
      </nav>
    </li>;
  }
  return <aside className="sidebar" aria-label="邮箱导航">
    <div className="workspace-name"><span>我的工作区</span><span className="label">本机</span></div>
    <Button variant="primary" className="compose-button" disabled={!accounts.length} title={accounts.length ? undefined : '请先添加邮箱'} onClick={onCompose}><Plus size={18} />写邮件<span>N</span></Button>
    <div className="sidebar-scroll">
      <div className="sidebar-section-title"><span>常用</span><span className="local-hint" title="在账号文件夹右侧点击图钉，可固定到这里">快捷入口</span></div>
      <nav className="shortcut-list" aria-label="常用入口">
        {defaultShortcuts.map(entry => <NavigationRow key={navigationKey(entry.target)} entry={entry} active={view === 'mail' && isNavigationActive(query, entry.target)} messages={messages} onSelect={() => navigate(entry.target)} />)}
        {pinned.map(target => {
          const account = accounts.find(item => item.id === target.accountId);
          if (!account) return null;
          const entry = allEntries.get(navigationKey(target));
          return entry && account ? <NavigationRow key={navigationKey(target)} entry={entry} secondary={account.name} active={view === 'mail' && isNavigationActive(query, target)} messages={messages} onSelect={() => navigate(target)} pin={{ pinned: true, onToggle: () => togglePinned(target), accountName: account.name }} />
            : <div className="navigation-row stale-shortcut" key={navigationKey(target)}><span>已失效的常用入口</span><button type="button" className="navigation-pin is-pinned" aria-label="移除失效的常用入口" onClick={() => togglePinned(target)}><PinOff size={14} /></button></div>;
        })}
      </nav>
      <div className="sidebar-divider" />
      <div className="sidebar-section-title"><span>邮箱账号</span><button title="管理账号" aria-label="管理邮箱账号" onClick={onAccounts}><Plus size={14} /></button></div>
      {!accounts.length && <p className="navigation-empty">还没有邮箱账号，可点击上方加号添加。</p>}
      <ul className="account-tree">{accounts.map(accountTree)}</ul>
      {labels.length > 0 && <><div className="sidebar-divider" /><div className="sidebar-section-title"><span>本地标签</span></div><nav className="folder-list" aria-label="邮件标签">{labels.map(label => <button key={label.id} title={label.name} className={query.labelId === label.id && view === 'mail' ? 'is-active' : ''} aria-current={query.labelId === label.id && view === 'mail' ? 'page' : undefined} onClick={() => { setQuery({ accountId: label.accountId ?? null, folderId: null, draftSource: null, labelId: label.id, search: '' }); onNavigate?.(); }}><span className={`label-dot tone-${label.color}`} /><span>{label.name}</span></button>)}</nav></>}
      {error && <p className="navigation-error" role="status">{error}</p>}
    </div>
    <div className="sidebar-note"><span className="status-dot" /><div><strong>本地优先</strong><p>{isDesktop() ? accounts.length ? `${accounts.length} 个邮箱` : '尚未添加邮箱' : '演示工作区 · 未连接真实邮箱'}</p></div></div>
  </aside>;
}
