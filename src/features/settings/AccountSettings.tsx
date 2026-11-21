import { useContext, useState } from 'react';
import { ArrowLeft, ChevronRight, Mail, Plus, RefreshCw, Trash2 } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { Avatar } from '../../components/ui/Avatar';
import { Button } from '../../components/ui/Button';
import { InlineConfirmation } from '../../components/ui/InlineConfirmation';
import { errorMessage } from '../../domain/errors';
import type { MailAccount } from '../../domain/mail';
import { isSyncActive, syncableAccounts, syncJobSummary } from '../../domain/sync';
import { useI18n, useLanguage } from '../../i18n/react';
import { desktopAccounts, isDesktop, type RealAccountInput } from '../../platform/desktop';
import { useSyncStore } from '../../stores/syncStore';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { AccountWizard } from './AccountWizard';
import { FolderManager } from './FolderManager';
import { ImapOptions } from './ImapOptions';
import { OAuthOptions } from './OAuthOptions';
import { SettingsPageHeading } from './SettingsPageHeading';
import { SignatureOptions } from './SignatureOptions';
import { SettingsNavigation, useSettingsNavigation } from './settingsNavigation';

type AccountSection = 'sync' | 'folders' | 'signature' | 'connection';
type AccountPage = { kind: 'list' } | { kind: 'add' } | { kind: 'detail'; id: string; section: AccountSection };

function protocol(account: MailAccount) {
  switch (account.provider) {
    case 'gmail': return 'Gmail';
    case 'microsoft': return 'Microsoft 365';
    case 'pop3': return 'POP3';
    default: return 'IMAP';
  }
}

export function AccountSettings({ initialAdd = false }: { initialAdd?: boolean }) {
  const t = useI18n();
  const language = useLanguage();
  const navigate = useContext(SettingsNavigation)!.request;
  const { accounts, reload, syncStates } = useWorkspaceStore(useShallow(s => ({ accounts: s.accounts, reload: s.reload, syncStates: s.syncStates })));
  const { jobs, submitting, syncError, enqueue, cancel, refresh } = useSyncStore(useShallow(s => ({
    jobs: s.jobs, submitting: s.submitting, syncError: s.error, enqueue: s.enqueue, cancel: s.cancel, refresh: s.refresh,
  })));
  const [page, setPage] = useState<AccountPage>({ kind: initialAdd ? 'add' : 'list' });
  const [focusList, setFocusList] = useState(false);
  const desktop = isDesktop();
  const selected = page.kind === 'detail' ? accounts.find(account => account.id === page.id) : undefined;
  const details = [
    { id: 'sync', label: t('syncOptions.button') },
    { id: 'folders', label: t('folders.button') },
    { id: 'signature', label: t('signature.button') },
    { id: 'connection', label: t('settings.connectionSettings') },
  ] as const;
  const eligible = syncableAccounts(accounts);
  const jobsByAccount = new Map(jobs.map(job => [job.accountId, job]));
  const formatTime = new Intl.DateTimeFormat(language, { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });

  function showList() {
    setPage({ kind: 'list' });
    setFocusList(true);
  }

  if (page.kind === 'add' || (page.kind === 'detail' && selected)) {
    return <>
      <Button variant="ghost" className="settings-back" onClick={() => navigate(showList)}><ArrowLeft size={16} aria-hidden />{t('settings.backToAccounts')}</Button>
      {page.kind === 'add' ? <AccountWizard tone={(['teal', 'blue', 'amber'] as const)[eligible.length % 3]} onSaved={reload} onComplete={showList} /> : selected && <>
        <SettingsPageHeading title={selected.name} description={selected.email} focus />
        <nav className="settings-subnav" aria-label={t('settings.accountSections')}>
          {details.filter(section => section.id !== 'folders' || selected.provider === 'imap').map(section => <Button key={section.id} variant="ghost" aria-current={page.section === section.id ? 'page' : undefined} onClick={() => {
            if (page.section !== section.id) navigate(() => setPage({ ...page, section: section.id }));
          }}>{section.label}</Button>)}
        </nav>
        <div key={`${selected.id}:${page.section}`} className="settings-detail">
          {page.section === 'sync' && <ImapOptions accountId={selected.id} provider={selected.provider} />}
          {page.section === 'folders' && <FolderManager accountId={selected.id} />}
          {page.section === 'signature' && <SignatureOptions accountId={selected.id} />}
          {page.section === 'connection' && <ConnectionOptions account={selected} reload={reload} onRemoved={showList} />}
        </div>
      </>}
    </>;
  }

  return <>
    <SettingsPageHeading title={t('settings.tabAccounts')} description={desktop ? t('settings.accountsDesktopHint') : t('settings.addAccountDesktopOnly')} focus={focusList} actions={<>
      {desktop && eligible.length > 1 && <Button disabled={!eligible.some(account => !submitting.includes(account.id) && !jobs.some(job => job.accountId === account.id && isSyncActive(job)))} onClick={() => void enqueue(eligible.map(account => account.id))}><RefreshCw size={15} aria-hidden />{t('settings.syncAll')}</Button>}
      <Button variant="primary" disabled={!desktop} onClick={() => setPage({ kind: 'add' })}><Plus size={16} aria-hidden />{t('settings.addAccount')}</Button>
    </>} />
    {accounts.length === 0 ? <div className="settings-empty">
      <Mail size={24} aria-hidden /><h4>{t('settings.noAccounts')}</h4><p>{t('app.firstAccountBody')}</p>
    </div> : <ul className="settings-accounts">
      {accounts.map(account => {
        const job = jobsByAccount.get(account.id);
        const active = job && isSyncActive(job);
        const isSubmitting = submitting.includes(account.id);
        const lastSync = syncStates?.[account.id];
        const status = account.status === 'demo' ? t('settings.statusDemo') : account.status === 'connected' ? t('settings.statusConnected') : account.status === 'offline' ? t('settings.statusOffline') : t('settings.statusReauth');
        return <li key={account.id} className="settings-account">
          <Avatar text={account.avatar} color={account.color} />
          <div className="settings-account-identity">
            <strong>{account.name}</strong>
            <small>{account.email}</small>
            {account.status !== 'demo' && <small className="sync-note">{job ? syncJobSummary(job) : lastSync ? t('settings.lastSync', { time: formatTime.format(new Date(lastSync)) }) : t('settings.neverSynced')}</small>}
          </div>
          <span className="settings-account-status" data-status={account.status}>{protocol(account)} · {status}</span>
          {desktop && account.status !== 'demo' && <div className="settings-actions">
            {active ? <Button variant="ghost" aria-label={job.status === 'running' ? t('settings.stopSyncAria', { name: account.name }) : t('settings.cancelQueuedAria', { name: account.name })} disabled={job.cancelRequested} onClick={() => void cancel(job.id)}>{job.status === 'running' ? t('settings.stopSync') : t('settings.cancelQueued')}</Button> : <Button variant="ghost" aria-label={t('settings.syncAria', { name: account.name })} disabled={isSubmitting} onClick={() => void enqueue([account.id])}>{isSubmitting ? t('settings.queuing') : job?.status === 'failed' ? t('settings.retry') : t('settings.sync')}</Button>}
            <Button aria-label={t('settings.manageAccountAria', { name: account.name })} onClick={() => setPage({ kind: 'detail', id: account.id, section: 'sync' })}>{t('settings.manageAccount')}<ChevronRight size={15} aria-hidden /></Button>
          </div>}
        </li>;
      })}
    </ul>}
    {jobs.some(isSyncActive) && <p className="sync-queue-note" role="status">{t('settings.queueNote', { running: jobs.filter(job => job.status === 'running').length, queued: jobs.filter(job => job.status === 'queued').length })}</p>}
    {syncError && <div className="inline-error" role="alert">{syncError} <Button variant="ghost" onClick={() => void refresh()}>{t('settings.reloadProgress')}</Button></div>}
  </>;
}

function ConnectionOptions({ account, reload, onRemoved }: { account: MailAccount; reload: () => Promise<void>; onRemoved: () => void }) {
  const t = useI18n();
  const [configuration, setConfiguration] = useState<RealAccountInput>();
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useSettingsNavigation(false, busy);

  async function configure() {
    setBusy(true);
    setError('');
    try { setConfiguration(await desktopAccounts.configuration(account.id)); }
    catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }

  async function remove() {
    setConfirmRemove(false);
    setBusy(true);
    setError('');
    try { await desktopAccounts.remove(account.id); await reload(); onRemoved(); }
    catch (error) { setError(t('settings.deleteFailed', { name: account.name, error: errorMessage(error) })); }
    finally { setBusy(false); }
  }

  if (configuration) {
    const onComplete = () => setConfiguration(undefined);
    return account.oauth ? <OAuthOptions account={configuration} initial={account.oauth} onSaved={reload} onComplete={onComplete} /> : <AccountWizard initial={configuration} tone={account.color} onSaved={reload} onComplete={onComplete} />;
  }

  return <>
    <SettingsPageHeading level="h4" title={t('settings.connectionSettings')} description={t('settings.connectionHint')} />
    <Button variant="primary" disabled={busy} aria-label={t('settings.reconfigAria', { name: account.name })} onClick={() => void configure()}>{account.oauth ? t('settings.reauthorize') : t('settings.editConnection')}</Button>
    {busy && <p role="status" className="muted">{t('settings.operationInProgress')}</p>}
    {error && <p className="inline-error" role="alert">{error}</p>}
    <div className="settings-danger-zone">
      {confirmRemove ? <InlineConfirmation message={t('settings.deleteAccountConfirm', { name: account.name })} confirmLabel={t('settings.deleteAccount')} onConfirm={() => void remove()} onCancel={() => setConfirmRemove(false)} /> : <Button variant="ghost" className="danger-button" disabled={busy} aria-label={t('settings.deleteAccountAria', { name: account.name })} onClick={() => setConfirmRemove(true)}><Trash2 size={15} aria-hidden />{t('settings.deleteAccount')}</Button>}
    </div>
  </>;
}
