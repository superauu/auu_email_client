import { ArrowDown, Star } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { mailboxNames, selectMessages } from './selectors';
import { EmptyState } from '../../components/ui/EmptyState';
import { cn } from '../../lib/cn';

export function MailList({ onSelect }: { onSelect: () => void }) {
  const { messages, accounts, labels, query, selectedId, savingIds, selectMessage, toggleStar, setQuery } = useWorkspaceStore(useShallow(s => ({
    messages: s.messages, accounts: s.accounts, labels: s.labels, query: s.query, selectedId: s.selectedId,
    savingIds: s.savingIds, selectMessage: s.selectMessage, toggleStar: s.toggleStar, setQuery: s.setQuery,
  })));
  const visible = selectMessages(messages, query);
  const queriedAccount = query.accountId ? accounts.find(a => a.id === query.accountId) : null;
  const realAccount = queriedAccount && queriedAccount.status !== 'demo' ? queriedAccount : null;
  const title = query.labelId ? labels.find(l => l.id === query.labelId)?.name : mailboxNames[query.mailbox];
  const folderTitle = useWorkspaceStore(state => state.folders)?.find(folder => folder.id === query.folderId)?.name;
  return <section className="mail-list" aria-label="邮件列表">
    <h1 className="sr-only">{folderTitle ?? title} {visible.length}</h1>
    <header className="list-toolbar">
      <div className="list-filters" role="group" aria-label="邮件阅读状态筛选">
        <button aria-pressed={!query.unreadOnly} onClick={() => setQuery({ unreadOnly: false })}>全部</button>
        <button aria-pressed={Boolean(query.unreadOnly)} onClick={() => setQuery({ unreadOnly: true })}>未读</button>
      </div>
      <span className="list-order" title="按邮件日期排序，最新邮件在前">按日期<ArrowDown size={12} /></span>
    </header>
    <div className="mail-list-scroll">
      {visible.length === 0 ? (query.unreadOnly ? <EmptyState title="没有未读邮件" description="当前筛选范围内的邮件都已读，可切回全部查看。" /> : query.search ? <EmptyState title="没有找到相关邮件" description="换个关键词，或选择其他账号试试。" /> : realAccount ? <EmptyState title={`${realAccount.name} 暂无缓存邮件`} description="请在邮箱账号设置中同步；已保存的草稿可以离线编辑。" /> : <EmptyState title="这里暂时没有邮件" description="收到的邮件和保存的草稿会在这里出现。" />) : visible.map(message => {
        const account = accounts.find(a => a.id === message.accountId);
        return <article key={message.id} className={cn('mail-row', selectedId === message.id && 'is-selected', message.unread && 'is-unread')}>
          <button className="mail-row-main" title={[message.from.email, account?.name, ...message.labelIds.map(id => labels.find(label => label.id === id)?.name)].filter(Boolean).join(' · ')} aria-label={`阅读邮件：${message.subject}`} onClick={() => { selectMessage(message.id); onSelect(); }} aria-current={selectedId === message.id ? 'true' : undefined}>
            <div className="mail-row-top"><span className="sender"><span className={cn('unread-dot', !message.unread && 'invisible')} /><span>{message.from.name}</span></span><time dateTime={message.receivedAt}>{new Intl.DateTimeFormat('zh-CN', { month: 'numeric', day: 'numeric' }).format(new Date(message.receivedAt))}</time></div>
            <div className="mail-row-subject">{message.subject}</div><p>{message.preview || '尚未填写正文'}</p>
          </button>
          {!query.accountId && <span className={`row-account tone-${account?.color ?? 'teal'}`} title={account?.email}>{account?.name}</span>}
          <button className={cn('row-star', message.starred && 'is-starred')} aria-label={`${message.starred ? '取消星标' : '添加星标'}：${message.subject}`} disabled={savingIds.includes(message.id) || Boolean(message.draft)} aria-pressed={message.starred} onClick={() => toggleStar(message.id)}><Star size={14} fill={message.starred ? 'currentColor' : 'none'} /></button>
        </article>;
      })}
    </div>
    <footer className="list-footer">共 {visible.length} 封邮件<span>{realAccount || visible.some(message => accounts.some(account => account.id === message.accountId && account.status !== 'demo')) ? '本地缓存' : '仅本机演示'}</span></footer>
  </section>;
}
