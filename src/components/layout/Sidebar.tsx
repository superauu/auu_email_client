import { Archive, ChevronDown, FilePenLine, Inbox, Plus, Send, Star, Trash2 } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { Button } from '../ui/Button';
import { Avatar } from '../ui/Avatar';
import { mailboxNames } from '../../features/mail/selectors';
import type { MailboxKind } from '../../domain/mail';

const folders: { id: MailboxKind; Icon: typeof Inbox }[] = [
  { id: 'inbox', Icon: Inbox }, { id: 'starred', Icon: Star }, { id: 'sent', Icon: Send },
  { id: 'drafts', Icon: FilePenLine }, { id: 'archive', Icon: Archive }, { id: 'trash', Icon: Trash2 },
];
export function Sidebar({ onCompose, onAccounts }: { onCompose: () => void; onAccounts: () => void }) {
  const { accounts, messages, labels, query, setQuery, view } = useWorkspaceStore(useShallow(s => ({ accounts: s.accounts, messages: s.messages, labels: s.labels, query: s.query, setQuery: s.setQuery, view: s.view })));
  const unread = (id: string | null) => messages.filter(m => (!id || m.accountId === id) && m.unread && m.mailbox === 'inbox').length;
  return <aside className="sidebar" aria-label="邮箱导航"><div className="workspace-name"><span>我的工作区</span><span className="label">PERSONAL</span></div><Button variant="primary" className="compose-button" onClick={onCompose}><Plus size={18} />写邮件<span>N</span></Button>
    <div className="sidebar-scroll"><div className="sidebar-section-title"><span>邮箱账号</span><button title="管理账号" aria-label="管理邮箱账号" onClick={onAccounts}><Plus size={14} /></button></div>
      <div className="account-list"><button className={`account-row ${!query.accountId ? 'is-active' : ''}`} aria-pressed={!query.accountId} onClick={() => setQuery({ accountId: null })}><span className="all-avatar"><Inbox size={17} /></span><span>全部账号</span><small>{unread(null)}</small></button>{accounts.map(account => <button className={`account-row ${query.accountId === account.id ? 'is-active' : ''}`} key={account.id} aria-pressed={query.accountId === account.id} onClick={() => setQuery({ accountId: account.id })}><Avatar text={account.avatar} color={account.color} small /><span>{account.name}</span>{unread(account.id) > 0 && <small>{unread(account.id)}</small>}</button>)}</div>
      <div className="sidebar-divider" /><div className="sidebar-section-title"><span>邮件</span><ChevronDown size={13} /></div>
      <nav className="folder-list" aria-label="邮件文件夹">{folders.map(({ id, Icon }) => { const count = messages.filter(m => (!query.accountId || query.accountId === m.accountId) && (id === 'starred' ? m.starred && m.mailbox !== 'trash' : m.mailbox === id)).length; return <button key={id} className={view === 'mail' && !query.labelId && query.mailbox === id ? 'is-active' : ''} aria-current={view === 'mail' && !query.labelId && query.mailbox === id ? 'page' : undefined} onClick={() => setQuery({ mailbox: id, labelId: null, search: '' })}><Icon size={17} /><span>{mailboxNames[id]}</span>{count > 0 && <small>{count}</small>}</button>; })}</nav>
      <div className="sidebar-divider" /><div className="sidebar-section-title"><span>我的标签</span><span className="local-hint">本地</span></div><nav className="folder-list" aria-label="邮件标签">{labels.map(label => <button key={label.id} className={query.labelId === label.id && view === 'mail' ? 'is-active' : ''} aria-current={query.labelId === label.id && view === 'mail' ? 'page' : undefined} onClick={() => setQuery({ labelId: label.id, search: '' })}><span className={`label-dot tone-${label.color}`} /><span>{label.name}</span></button>)}</nav>
    </div><div className="sidebar-note"><span className="status-dot" /><div><strong>你的邮件，你的节奏</strong><p>演示工作区 · 未连接真实邮箱</p></div></div>
  </aside>;
}
