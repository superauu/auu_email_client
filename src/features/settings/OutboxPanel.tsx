import { useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { Button } from '../../components/ui/Button';
import { outboxSummary } from '../../domain/outbox';
import { isDesktop } from '../../platform/desktop';
import { useSyncStore } from '../../stores/syncStore';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { useI18n, currentLanguage } from '../../i18n/react';

export function OutboxPanel() {
  const t = useI18n();
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
    setNotice(t('outbox.returnedNotice'));
  }

  return <section className="outbox-panel" aria-label={t('outbox.panelAria')}>
    <div className="setting-heading"><h3>{t('outbox.heading')}</h3><Button variant="ghost" disabled={!isDesktop()} onClick={() => void refresh()}>{t('outbox.refresh')}</Button></div>
    <p className="muted">{t('outbox.intro')}</p>
    {isDesktop() ? <>
      <div className="outbox-filters">
        <label>{t('outbox.accountLabel')}<select aria-label={t('outbox.accountAria')} value={selectedAccount} onChange={event => setAccountId(event.target.value)}><option value="">{t('outbox.allAccounts')}</option>{realAccounts.map(account => <option key={account.id} value={account.id}>{account.name} · {account.email}</option>)}</select></label>
        <label>{t('outbox.statusLabel')}<select aria-label={t('outbox.statusAria')} value={filter} onChange={event => setFilter(event.target.value)}><option value="all">{t('outbox.allStates')}</option><option value="pending">{t('outbox.pending')}</option><option value="sent">{t('outbox.sent')}</option><option value="local">{t('reader.keepLocal')}</option></select></label>
      </div>
      {!loaded ? <p role="status">{t('outbox.loading')}</p> : !visible.length ? <p role="status">{t('outbox.empty')}</p> : <div className="outbox-list">
        {visible.map(entry => {
          const account = realAccounts.find(account => account.id === entry.accountId)!;
          const message = messages.find(message => message.id === entry.draftId && message.accountId === entry.accountId);
          const busy = pending.includes(entry.id);
          const failed = entry.state === 'failed' && entry.resolvedBy !== 'keep-local';
          const sendBlocked = !account.capabilities.send;
          return <article className="outbox-entry" key={entry.id} aria-label={t('outbox.entryAria', { subject: message?.subject || t('outbox.untitledDraft'), account: account.name })} aria-busy={busy}>
            <strong>{message?.subject || t('outbox.untitledDraft')}</strong>
            <span className="muted">{account.name} · {account.email}</span>
            <span role="status" className={failed ? 'outbox-failed' : undefined}>{failed ? t('outbox.failedAttempts', { attempts: entry.attempts }) : outboxSummary(entry)}</span>
            {entry.lastError && <p className="outbox-error">{entry.lastError}</p>}
            {entry.queuedAt && <time dateTime={entry.queuedAt}>{t('outbox.queuedAt', { time: new Intl.DateTimeFormat(currentLanguage(), { dateStyle: 'short', timeStyle: 'short' }).format(new Date(entry.queuedAt)) })}</time>}
            <div className="outbox-actions">
              {entry.state === 'queued' && <Button variant="secondary" disabled={busy} onClick={() => void release(entry.id, entry.draftId, entry.accountId, false)}>{t('outbox.cancelSend')}</Button>}
              {entry.state === 'sending' && <span className="muted">{t('outbox.sendingWait')}</span>}
              {failed && <><Button variant="secondary" disabled={busy || sendBlocked} title={sendBlocked ? t('outbox.sendBlockedTitle') : undefined} onClick={() => void retry(entry.id)}>{t('outbox.retrySend')}</Button><Button variant="ghost" disabled={busy} onClick={() => void release(entry.id, entry.draftId, entry.accountId, true)}>{t('outbox.returnToDraft')}</Button></>}
              {entry.state === 'unknown' && <><Button variant="ghost" disabled={busy} onClick={() => void resolve(entry.id, 'keep-local')}>{t('reader.keepLocal')}</Button><Button variant="secondary" disabled={busy || sendBlocked} title={sendBlocked ? t('outbox.sendBlockedTitle') : undefined} onClick={() => void resolve(entry.id, 'retry')}>{t('outbox.confirmNotDelivered')}</Button></>}
              {busy && <span role="status">{t('outbox.processing')}</span>}
            </div>
          </article>;
        })}
      </div>}
      {notice && <p role="status">{notice}</p>}
    </> : <p className="muted">{t('outbox.browserNote')}</p>}
  </section>;
}
