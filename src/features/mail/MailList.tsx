import { Search, Star } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { mailboxNames, selectMessages } from './selectors';
import { Avatar } from '../../components/ui/Avatar';
import { EmptyState } from '../../components/ui/EmptyState';
import { cn } from '../../lib/cn';

export function MailList({ onSelect }: { onSelect: () => void }) {
  const { messages, accounts, labels, query, selectedId, savingIds, selectMessage, toggleStar } = useWorkspaceStore(useShallow(s => ({
    messages: s.messages, accounts: s.accounts, labels: s.labels, query: s.query, selectedId: s.selectedId,
    savingIds: s.savingIds, selectMessage: s.selectMessage, toggleStar: s.toggleStar,
  })));
  const visible = selectMessages(messages, query);
  const queriedAccount = query.accountId ? accounts.find(a => a.id === query.accountId) : null;
  const realAccount = queriedAccount && queriedAccount.status !== 'demo' ? queriedAccount : null;
  const title = query.labelId ? labels.find(l => l.id === query.labelId)?.name : mailboxNames[query.mailbox];
  const folderTitle = useWorkspaceStore(state => state.folders)?.find(folder => folder.id === query.folderId)?.name;
  return <section className="mail-list" aria-label="邮件列表">
    <header className="list-header"><div><span className="eyebrow">YOUR MAIL, IN ORDER</span><h1>{folderTitle ?? title}<span className="count-pill">{visible.length}</span></h1></div><span className="list-order">最新在前</span></header>
    <div className="list-context">{query.accountId ? accounts.find(a => a.id === query.accountId)?.email : '所有账号的邮件，尽在一处'}{query.search && <span><Search size={12} /> 搜索结果</span>}</div>
    <div className="mail-list-scroll">
      {visible.length === 0 ? (query.search ? <EmptyState title="没有找到相关邮件" description="换个关键词，或选择其他账号试试。" /> : realAccount ? <EmptyState title={`${realAccount.name} 暂无缓存邮件`} description="请在邮箱账号设置中同步；已保存的草稿可以离线编辑。" /> : <EmptyState title="这里暂时没有邮件" description="收到的邮件和保存的草稿会在这里出现。" />) : visible.map(message => {
        const account = accounts.find(a => a.id === message.accountId);
        return <article key={message.id} className={cn('mail-row', selectedId === message.id && 'is-selected', message.unread && 'is-unread')}>
          <button className="mail-row-main" aria-label={`阅读邮件：${message.subject}`} onClick={() => { selectMessage(message.id); onSelect(); }} aria-current={selectedId === message.id ? 'true' : undefined}>
            <div className="mail-row-top"><span className="sender"><span className={cn('unread-dot', !message.unread && 'invisible')} /><Avatar text={message.from.name.slice(0, 1)} color={account?.color} small /><span>{message.from.name}</span></span><time dateTime={message.receivedAt}>{new Intl.DateTimeFormat('zh-CN', { month: 'numeric', day: 'numeric' }).format(new Date(message.receivedAt))}</time></div>
            <div className="mail-row-subject">{message.subject}</div><p>{message.preview || '尚未填写正文'}</p>
            <div className="mail-row-meta"><span className={`account-indicator tone-${account?.color ?? 'teal'}`}>{account?.name}</span>{message.labelIds.slice(0, 1).map(id => <span className="tiny-label" key={id}>{labels.find(l => l.id === id)?.name}</span>)}</div>
          </button>
          <button className={cn('row-star', message.starred && 'is-starred')} aria-label={`${message.starred ? '取消星标' : '添加星标'}：${message.subject}`} disabled={savingIds.includes(message.id) || Boolean(message.draft)} aria-pressed={message.starred} onClick={() => toggleStar(message.id)}><Star size={14} fill={message.starred ? 'currentColor' : 'none'} /></button>
        </article>;
      })}
    </div>
    <footer className="list-footer">共 {visible.length} 封邮件<span>{realAccount || visible.some(message => accounts.some(account => account.id === message.accountId && account.status !== 'demo')) ? '本地缓存' : '仅本机演示'}</span></footer>
  </section>;
}
