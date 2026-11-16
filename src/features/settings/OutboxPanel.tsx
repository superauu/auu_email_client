import { useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { Button } from '../../components/ui/Button';
import { outboxSummary } from '../../domain/outbox';
import { isDesktop } from '../../platform/desktop';
import { useSyncStore } from '../../stores/syncStore';
import { useWorkspaceStore } from '../../stores/workspaceStore';

export function OutboxPanel() {
  const { accounts, messages, setQuery, selectMessage, setView } = useWorkspaceStore(useShallow(s => ({ accounts: s.accounts, messages: s.messages, setQuery: s.setQuery, selectMessage: s.selectMessage, setView: s.setView })));
  const { outbox, loaded, pending, refresh, cancel, retry, resolve, returnDraft } = useSyncStore(useShallow(s => ({ outbox: s.outbox, loaded: s.outboxLoaded, pending: s.outboxPending, refresh: s.refresh, cancel: s.outboxCancel, retry: s.outboxRetry, resolve: s.outboxResolve, returnDraft: s.outboxReturnDraft })));
  const [accountId, setAccountId] = useState('');
  const [filter, setFilter] = useState('all');
  const [notice, setNotice] = useState('');
  const realAccounts = accounts.filter(account => account.status !== 'demo');
  const selectedAccount = realAccounts.some(account => account.id === accountId) ? accountId : '';
  const visible = outbox.filter(entry => realAccounts.some(account => account.id === entry.accountId)
    && (!selectedAccount || entry.accountId === selectedAccount)
    && (filter === 'all' || (filter === 'local' ? entry.resolvedBy === 'keep-local' : filter === 'sent' ? entry.state === 'sent' : entry.state !== 'sent' && entry.resolvedBy !== 'keep-local')));

  async function release(id: string, draftId: string, owner: string, failed: boolean) {
    setNotice('');
    if (!await (failed ? returnDraft(id) : cancel(id))) return;
    setView('mail');
    setQuery({ accountId: owner, mailbox: 'drafts', draftSource: 'local', folderId: null, search: '', labelId: null, unreadOnly: false });
    selectMessage(draftId);
    setNotice('任务已返回本机草稿，关闭设置后可继续编辑；正文和附件均保留。');
  }

  return <section className="outbox-panel" aria-label="发送队列">
    <div className="setting-heading"><h3>发送队列</h3><Button variant="ghost" disabled={!isDesktop()} onClick={() => void refresh()}>刷新发送队列</Button></div>
    <p className="muted">显示全部未发送任务与最近 128 条已发送记录。结果不明不会自动重发；仅排队中的任务可取消。</p>
    {isDesktop() ? <>
      <div className="outbox-filters">
        <label>账号<select aria-label="发送队列账号" value={selectedAccount} onChange={event => setAccountId(event.target.value)}><option value="">全部账号</option>{realAccounts.map(account => <option key={account.id} value={account.id}>{account.name} · {account.email}</option>)}</select></label>
        <label>状态<select aria-label="发送队列状态" value={filter} onChange={event => setFilter(event.target.value)}><option value="all">全部状态</option><option value="pending">待处理</option><option value="sent">已发送</option><option value="local">仅保留本机</option></select></label>
      </div>
      {!loaded ? <p role="status">正在读取发送队列…</p> : !visible.length ? <p role="status">当前筛选下没有发送任务</p> : <div className="outbox-list">
        {visible.map(entry => {
          const account = realAccounts.find(account => account.id === entry.accountId)!;
          const message = messages.find(message => message.id === entry.draftId && message.accountId === entry.accountId);
          const busy = pending.includes(entry.id);
          const failed = entry.state === 'failed' && entry.resolvedBy !== 'keep-local';
          const sendBlocked = !account.capabilities.send;
          return <article className="outbox-entry" key={entry.id} aria-label={`发送任务：${message?.subject || '未命名草稿'} · ${account.name}`} aria-busy={busy}>
            <strong>{message?.subject || '未命名草稿'}</strong>
            <span className="muted">{account.name} · {account.email}</span>
            <span role="status" className={failed ? 'outbox-failed' : undefined}>{failed ? `发送失败（已尝试 ${entry.attempts} 次）` : outboxSummary(entry)}</span>
            {entry.lastError && <p className="outbox-error">{entry.lastError}</p>}
            {entry.queuedAt && <time dateTime={entry.queuedAt}>入队于 {new Intl.DateTimeFormat('zh-CN', { dateStyle: 'short', timeStyle: 'short' }).format(new Date(entry.queuedAt))}</time>}
            <div className="outbox-actions">
              {entry.state === 'queued' && <Button variant="secondary" disabled={busy} onClick={() => void release(entry.id, entry.draftId, entry.accountId, false)}>取消发送</Button>}
              {entry.state === 'sending' && <span className="muted">发送中，请等待服务器结果</span>}
              {failed && <><Button variant="secondary" disabled={busy || sendBlocked} title={sendBlocked ? '当前账号未开放 SMTP 发送，请先检查连接设置或授权。' : undefined} onClick={() => void retry(entry.id)}>重试发送</Button><Button variant="ghost" disabled={busy} onClick={() => void release(entry.id, entry.draftId, entry.accountId, true)}>返回草稿修改</Button></>}
              {entry.state === 'unknown' && <><Button variant="ghost" disabled={busy} onClick={() => void resolve(entry.id, 'keep-local')}>仅保留本机</Button><Button variant="secondary" disabled={busy || sendBlocked} title={sendBlocked ? '当前账号未开放 SMTP 发送，请先检查连接设置或授权。' : undefined} onClick={() => void resolve(entry.id, 'retry')}>确认未送达并重试</Button></>}
              {busy && <span role="status">正在处理…</span>}
            </div>
          </article>;
        })}
      </div>}
      {notice && <p role="status">{notice}</p>}
    </> : <p className="muted">浏览器演示不能发送邮件，请在桌面版查看发送队列。</p>}
  </section>;
}
