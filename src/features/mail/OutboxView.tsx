import { useEffect, useRef, useState } from 'react';
import { RefreshCw, Send } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { Button } from '../../components/ui/Button';
import { InlineConfirmation } from '../../components/ui/InlineConfirmation';
import { outboxSummary, type OutboxEntry } from '../../domain/outbox';
import { useI18n, useLanguage } from '../../i18n/react';
import { useSyncStore } from '../../stores/syncStore';
import { useWorkspaceStore } from '../../stores/workspaceStore';

export function OutboxView({ onDraftReleased }: { onDraftReleased: (entry: OutboxEntry) => void }) {
  const t = useI18n();
  const language = useLanguage();
  const heading = useRef<HTMLHeadingElement>(null);
  const { accounts, messages } = useWorkspaceStore(useShallow(s => ({ accounts: s.accounts, messages: s.messages })));
  const { outbox, loaded, loadError, actionError, pending, refresh, cancel, retry, resolve, returnDraft } = useSyncStore(useShallow(s => ({
    outbox: s.outbox, loaded: s.outboxLoaded, loadError: s.outboxLoadError, actionError: s.outboxActionError, pending: s.outboxPending,
    refresh: s.refresh, cancel: s.outboxCancel, retry: s.outboxRetry, resolve: s.outboxResolve, returnDraft: s.outboxReturnDraft,
  })));
  const [accountId, setAccountId] = useState('');
  const [filter, setFilter] = useState('pending');
  const [refreshing, setRefreshing] = useState(false);
  const [confirmRetry, setConfirmRetry] = useState<string | null>(null);
  const realAccounts = new Map(accounts.filter(account => account.status !== 'demo').map(account => [account.id, account]));
  const messagesById = new Map(messages.map(message => [message.id, message]));
  const selectedAccount = realAccounts.has(accountId) ? accountId : '';
  const formatTime = new Intl.DateTimeFormat(language, { dateStyle: 'short', timeStyle: 'short' });
  const visible = outbox.filter(entry => realAccounts.has(entry.accountId)
    && (!selectedAccount || entry.accountId === selectedAccount)
    && (filter === 'all' || (filter === 'local' ? entry.resolvedBy === 'keep-local' : filter === 'sent' ? entry.state === 'sent' : entry.state !== 'sent' && entry.resolvedBy !== 'keep-local')));

  useEffect(() => {
    heading.current?.focus();
    void refresh();
  }, [refresh]);

  async function refreshView() {
    setRefreshing(true);
    try { await refresh(); }
    finally { setRefreshing(false); }
  }

  async function release(entry: OutboxEntry, failed: boolean) {
    if (await (failed ? returnDraft(entry.id) : cancel(entry.id))) onDraftReleased(entry);
  }

  return <main className="outbox-view" aria-labelledby="outbox-heading">
    <section className="outbox-panel" aria-label={t('outbox.panelAria')}>
      <header className="outbox-header">
        <div>
          <h1 id="outbox-heading" ref={heading} tabIndex={-1}>{t('outbox.heading')}</h1>
          <p>{t('outbox.intro')}</p>
        </div>
        <Button disabled={refreshing} onClick={() => void refreshView()} aria-label={t('outbox.refresh')}><RefreshCw size={15} className={refreshing ? 'spin' : undefined} aria-hidden />{refreshing ? t('outbox.loading') : t('outbox.refreshButton')}</Button>
      </header>
      <div className="outbox-filters">
        <label>{t('outbox.statusLabel')}<select name="state" aria-label={t('outbox.statusAria')} value={filter} onChange={event => { setFilter(event.target.value); setConfirmRetry(null); }}>
          <option value="pending">{t('outbox.pending')}</option>
          <option value="sent">{t('outbox.sent')}</option>
          <option value="local">{t('reader.keepLocal')}</option>
          <option value="all">{t('outbox.allStates')}</option>
        </select></label>
        <label>{t('outbox.accountLabel')}<select name="account" aria-label={t('outbox.accountAria')} value={selectedAccount} onChange={event => { setAccountId(event.target.value); setConfirmRetry(null); }}>
          <option value="">{t('outbox.allAccounts')}</option>
          {Array.from(realAccounts.values()).map(account => <option key={account.id} value={account.id}>{account.name} · {account.email}</option>)}
        </select></label>
      </div>
      {loadError && <p className="inline-error" role="alert">{t('outbox.loadFailed', { error: loadError })}</p>}
      {actionError && <p className="inline-error" role="alert">{actionError}</p>}
      {!loaded && !loadError && <p className="muted" role="status">{t('outbox.loading')}</p>}
      {loaded && visible.length === 0 && <div className="outbox-empty">
        <span><Send size={22} aria-hidden /></span>
        <div><h2>{filter === 'pending' && !selectedAccount ? t('outbox.emptyPending') : t('outbox.empty')}</h2>
          {filter === 'pending' && !selectedAccount && <p>{t('outbox.emptyPendingHint')}</p>}</div>
      </div>}
      {loaded && visible.length > 0 && <ul className="outbox-list">
        {visible.map(entry => {
          const account = realAccounts.get(entry.accountId)!;
          const message = messagesById.get(entry.draftId);
          const subject = (message?.accountId === account.id && message.subject) || t('outbox.untitledDraft');
          const busy = pending.includes(entry.id);
          const failed = entry.state === 'failed' && entry.resolvedBy !== 'keep-local';
          const sendBlocked = !account.capabilities.send;
          const actionable = entry.state === 'queued' || entry.state === 'unknown' || failed || busy;
          return <li key={entry.id}><article className="outbox-entry" aria-label={t('outbox.entryAria', { subject, account: account.name })} aria-busy={busy}>
            <div className="outbox-entry-heading">
              <div>
                <h2>{subject}</h2>
                <p className="muted">{account.name} · {account.email}</p>
                {entry.queuedAt && <time dateTime={entry.queuedAt}>{t('outbox.queuedAt', { time: formatTime.format(new Date(entry.queuedAt)) })}</time>}
              </div>
              <span role="status" className="outbox-state" data-state={failed ? 'failed' : entry.resolvedBy === 'keep-local' ? 'local' : entry.state}>{failed ? t('outbox.failedAttempts', { attempts: entry.attempts }) : outboxSummary(entry)}</span>
            </div>
            {entry.lastError && <p className="outbox-error">{entry.lastError}</p>}
            {actionable && <div className="outbox-actions">
              {entry.state === 'queued' && <Button disabled={busy} onClick={() => void release(entry, false)}>{t('outbox.cancelSend')}</Button>}
              {failed && <><Button disabled={busy || sendBlocked} title={sendBlocked ? t('outbox.sendBlockedTitle') : undefined} onClick={() => void retry(entry.id)}>{t('outbox.retrySend')}</Button><Button variant="ghost" disabled={busy} onClick={() => void release(entry, true)}>{t('outbox.returnToDraft')}</Button></>}
              {entry.state === 'unknown' && confirmRetry !== entry.id && <><Button variant="ghost" disabled={busy} onClick={() => void resolve(entry.id, 'keep-local')}>{t('reader.keepLocal')}</Button><Button disabled={busy || sendBlocked} title={sendBlocked ? t('outbox.sendBlockedTitle') : undefined} onClick={() => setConfirmRetry(entry.id)}>{t('outbox.reviewRetry')}</Button></>}
              {entry.state === 'unknown' && confirmRetry === entry.id && !sendBlocked && <InlineConfirmation message={t('outbox.retryConfirm')} confirmLabel={t('outbox.confirmNotDelivered')} onCancel={() => setConfirmRetry(null)} onConfirm={() => { setConfirmRetry(null); void resolve(entry.id, 'retry'); }} />}
              {sendBlocked && (failed || entry.state === 'unknown') && <p className="muted">{t('outbox.sendBlockedTitle')}</p>}
              {busy && <span role="status">{t('outbox.processing')}</span>}
            </div>}
          </article></li>;
        })}
      </ul>}
    </section>
  </main>;
}
