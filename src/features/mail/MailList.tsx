import { ArrowDown, RefreshCw, Star } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useSyncStore } from '../../stores/syncStore';
import { mailboxNames, selectMessages } from './selectors';
import { Button } from '../../components/ui/Button';
import { EmptyState } from '../../components/ui/EmptyState';
import { cn } from '../../lib/cn';
import { isDesktop } from '../../platform/desktop';
import { isSyncActive, syncableAccounts } from '../../domain/sync';
import { directoryLabel } from '../../domain/navigation';

export function MailList({ onSelect }: { onSelect: () => void }) {
  const { messages, accounts, labels, query, selectedId, savingIds, selectMessage, toggleStar, setQuery } = useWorkspaceStore(useShallow(s => ({
    messages: s.messages, accounts: s.accounts, labels: s.labels, query: s.query, selectedId: s.selectedId,
    savingIds: s.savingIds, selectMessage: s.selectMessage, toggleStar: s.toggleStar, setQuery: s.setQuery,
  })));
  const { jobs, submitting, enqueue } = useSyncStore(useShallow(s => ({ jobs: s.jobs, submitting: s.submitting, enqueue: s.enqueue })));
  const jobsByAccount = new Map(jobs.map(job => [job.accountId, job]));
  const visible = selectMessages(messages, query);
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
  const title = query.labelId ? labels.find(l => l.id === query.labelId)?.name : query.draftSource === 'local' ? '本机草稿' : query.draftSource === 'server' ? '服务器草稿' : !query.accountId && query.mailbox === 'inbox' ? '全部收件箱' : mailboxNames[query.mailbox];
  const folders = useWorkspaceStore(state => state.folders) ?? [];
  const folder = folders.find(item => item.id === query.folderId && item.accountId === query.accountId);
  const folderTitle = folder ? directoryLabel(folder, folders.filter(item => item.accountId === query.accountId)) : undefined;
  return <section className="mail-list" aria-label="邮件列表">
    <h1 className="sr-only">{folderTitle ?? title} {visible.length}</h1>
    <header className="list-toolbar">
      <div className="list-filters" role="group" aria-label="邮件阅读状态筛选">
        <button aria-pressed={!query.unreadOnly} onClick={() => setQuery({ unreadOnly: false })}>全部</button>
        <button aria-pressed={Boolean(query.unreadOnly)} onClick={() => setQuery({ unreadOnly: true })}>未读</button>
      </div>
      <div className="list-actions"><span className="list-order" title="按邮件日期排序，最新邮件在前">按日期<ArrowDown size={12} /></span><Button variant="ghost" className="account-sync" aria-label={syncTarget ? `同步 ${syncTarget.name}` : '同步全部账号'} title={syncTarget ? `同步 ${syncTarget.name}` : '同步全部账号'} disabled={!isDesktop() || !syncable.length || (syncTarget ? accountSyncing : syncingAll)} onClick={syncCurrent}><RefreshCw size={14} className={(syncTarget ? accountSyncing : syncingAll) ? 'spin' : ''} />{syncTarget ? submitting.includes(syncTarget.id) ? '正在排队…' : jobsByAccount.get(syncTarget.id)?.status === 'queued' ? '排队中' : jobsByAccount.get(syncTarget.id)?.status === 'running' ? '同步中…' : jobsByAccount.get(syncTarget.id)?.status === 'failed' ? '重试' : '同步' : syncingAll ? '同步中…' : '同步'}</Button></div>
    </header>
    <div className="mail-list-scroll">
      {visible.length === 0 ? (query.unreadOnly ? <EmptyState title="没有未读邮件" description="当前筛选范围内的邮件都已读，可切回全部查看。" /> : query.search ? <EmptyState title="没有找到相关邮件" description="换个关键词，或选择其他账号试试。" /> : realAccount ? <EmptyState title={`${realAccount.name} 暂无缓存邮件`} description="点工具栏的同步按钮拉取邮件；已保存的草稿可以离线编辑。" /> : <EmptyState title="这里暂时没有邮件" description="收到的邮件和保存的草稿会在这里出现。" />) : visible.map(message => {
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
    <footer className="list-footer">共 {visible.length} 封邮件<span>{isDesktop() ? '本地缓存' : '仅本机演示'}</span></footer>
  </section>;
}
