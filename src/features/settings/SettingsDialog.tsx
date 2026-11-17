import { useEffect, useState } from 'react';
import * as Tabs from '@radix-ui/react-tabs';
import { Check, FolderOpen, Languages, Monitor, Moon, Plus, RefreshCw, Sun, Trash2 } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { Dialog } from '../../components/ui/Dialog';
import { Button } from '../../components/ui/Button';
import { Avatar } from '../../components/ui/Avatar';
import { useThemeStore } from '../../stores/themeStore';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { desktop, desktopAccounts, isDesktop, type AppInfo, type RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { resolveSkin, type ThemeMode } from '../../themes/contract';
import { AccountWizardDialog } from './AccountWizardDialog';
import { OAuthDialog } from './OAuthDialog';
import type { OAuthConfig } from '../../domain/oauth';
import { ImapOptions } from './ImapOptions';
import { useSyncStore } from '../../stores/syncStore';
import { isSyncActive, syncJobSummary, syncableAccounts } from '../../domain/sync';
import { OutboxPanel } from './OutboxPanel';
import { useI18n, useLanguage, currentLanguage, languageLabel, setLanguage, languages } from '../../i18n/react';

export function SettingsDialog({ open, onClose, initialTab }: { open: boolean; onClose: () => void; initialTab: 'appearance' | 'accounts' | 'about' }) {
  const t = useI18n();
  const language = useLanguage();
  const { skins, skinId, mode, warnings, loading, setAppearance, rescan } = useThemeStore(useShallow(s => ({ skins: s.skins, skinId: s.skinId, mode: s.mode, warnings: s.warnings, loading: s.loading, setAppearance: s.setAppearance, rescan: s.rescan })));
  const { accounts, reload, syncStates } = useWorkspaceStore(useShallow(s => ({ accounts: s.accounts, reload: s.reload, syncStates: s.syncStates })));
  const [error, setError] = useState('');
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [editing, setEditing] = useState<RealAccountInput | undefined>();
  const [reauthorizing, setReauthorizing] = useState<{account: RealAccountInput; config: OAuthConfig} | null>(null);
  const [configuring, setConfiguring] = useState(false);
  async function configure(id: string, oauth?: OAuthConfig | null) { setError(''); setConfiguring(true); try { const account = await desktopAccounts.configuration(id); if (oauth) setReauthorizing({ account, config: oauth }); else { setEditing(account); setWizardOpen(true); } } catch (error) { setError(errorMessage(error)); } finally { setConfiguring(false); } }
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const { jobs, submitting, syncError, enqueue, cancel, refreshSync } = useSyncStore(useShallow(s => ({ jobs: s.jobs, submitting: s.submitting, syncError: s.error, enqueue: s.enqueue, cancel: s.cancel, refreshSync: s.refresh })));
  useEffect(() => { if (open) void desktop.info().then(setInfo).catch(e => setError(errorMessage(e))); }, [open]);
  useEffect(() => {
    if (!pendingDelete) return;
    const timeout = window.setTimeout(() => setPendingDelete(null), 3000);
    return () => window.clearTimeout(timeout);
  }, [pendingDelete]);
  async function removeAccount(id: string, name: string) {
    if (pendingDelete !== id) { setPendingDelete(id); return; }
    setPendingDelete(null); setError('');
    try { await desktopAccounts.remove(id); await reload(); }
    catch (removeError) { setError(t('settings.deleteFailed', { name, error: errorMessage(removeError) })); }
  }
  const eligible = syncableAccounts(accounts);
  const jobsByAccount = new Map(jobs.map(job => [job.accountId, job]));
  const realCount = accounts.filter(account => account.status !== 'demo').length;
  const nextTone = realCount % 3 === 0 ? 'teal' : realCount % 3 === 1 ? 'blue' : 'amber' as const;
  const lastSync = (id: string) => syncStates?.[id] ? new Intl.DateTimeFormat(currentLanguage(), { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(new Date(syncStates[id])) : null;
  const statusLabel = (status: string) => status === 'demo' ? t('settings.statusDemo') : status === 'connected' ? t('settings.statusConnected') : status === 'offline' ? t('settings.statusOffline') : t('settings.statusReauth');
  return <Dialog open={open} onOpenChange={value => { if (!value) onClose(); }} title={t('settings.title')} description={t('settings.description')} className="settings-dialog">
    <Tabs.Root defaultValue={initialTab} className="settings-tabs"><Tabs.List className="tab-list" aria-label={t('settings.tabsAria')}><Tabs.Trigger value="appearance">{t('settings.tabAppearance')}</Tabs.Trigger><Tabs.Trigger value="accounts">{t('settings.tabAccounts')}</Tabs.Trigger><Tabs.Trigger value="about">{t('settings.tabAbout')}</Tabs.Trigger></Tabs.List>
      <Tabs.Content value="appearance" className="settings-body"><div className="setting-heading"><h3>{t('settings.pickSkin')}</h3><span>{t('settings.skinHint')}</span></div><div className="skin-grid">{skins.map(skin => {
        const sample = resolveSkin([skin], skin.manifest.id, mode, window.matchMedia('(prefers-color-scheme: dark)').matches);
        const selected = skinId === skin.manifest.id;
        return <button key={skin.manifest.id} className={`skin-card ${selected ? 'is-active' : ''}`} aria-pressed={selected} aria-label={t('settings.skinAria', { name: skin.manifest.name })} onClick={() => setAppearance(skin.manifest.id, mode)}>
          <span className="skin-preview" style={{ background: sample.tokens.canvas, borderColor: sample.tokens.border }}><span style={{ background: sample.tokens.rail }} /><span style={{ background: sample.tokens.surface }}><i style={{ background: sample.tokens.accent }} /><i style={{ background: sample.tokens.border }} /><i style={{ background: sample.tokens.border }} /></span><span style={{ background: sample.tokens.surface }}><i style={{ background: sample.tokens.text, opacity: 0.65 }} /><i style={{ background: sample.tokens.border }} /><i style={{ background: sample.tokens.border }} /></span></span>
          <span className="skin-title">{skin.manifest.name}{selected && <Check size={15} />}</span><small>{skin.source === 'builtin' ? t('settings.builtinSkin') : t('settings.skinBy', { author: skin.manifest.author })}</small>
        </button>;
      })}</div>
      <div className="setting-heading"><h3>{t('settings.modeHeading')}</h3><span>{t('settings.modeHint')}</span></div><div className="mode-options">{([{ id: 'light', Icon: Sun, label: t('settings.modeLight') }, { id: 'dark', Icon: Moon, label: t('settings.modeDark') }, { id: 'system', Icon: Monitor, label: t('settings.modeSystem') }] as const).map(({ id, Icon, label }) => <Button key={id} variant={mode === id ? 'primary' : 'secondary'} aria-pressed={mode === id} onClick={() => setAppearance(skinId, id as ThemeMode)}><Icon size={16} />{label}</Button>)}</div>
      <div className="setting-heading"><h3>{t('settings.languageHeading')}</h3><span>{t('settings.languageHint')}</span></div><div className="mode-options">{languages.map(id => <Button key={id} variant={language === id ? 'primary' : 'secondary'} aria-pressed={language === id} onClick={() => setLanguage(id)}><Languages size={16} />{languageLabel(id)}</Button>)}</div>
      <div className="skin-directory"><div><h3>{t('settings.customSkinHeading')}</h3><p>{isDesktop() ? t('settings.skinDesktopHint') : t('settings.skinBrowserHint')}</p>{info && <code>{info.skinDirectory}</code>}</div><div><Button disabled={!isDesktop()} onClick={() => { setError(''); void desktop.openSkinDirectory().catch(e => setError(errorMessage(e))); }}><FolderOpen size={15} />{t('settings.openSkinDir')}</Button><Button onClick={() => void rescan()} disabled={loading}><RefreshCw size={15} className={loading ? 'spin' : ''} />{loading ? t('settings.scanning') : t('settings.rescan')}</Button></div></div>
      {warnings.length > 0 && <div className="warning-list" role="status">{warnings.map((warning, index) => <p key={index}>{warning}</p>)}</div>}
      </Tabs.Content>
      <Tabs.Content value="accounts" className="settings-body"><div className="setting-heading"><h3>{t('settings.accountsHeading')}</h3><div className="account-actions"><Button disabled={!isDesktop() || !eligible.some(account => !submitting.includes(account.id) && !(jobsByAccount.get(account.id) && isSyncActive(jobsByAccount.get(account.id)!)))} onClick={() => void enqueue(eligible.map(account => account.id))}><RefreshCw size={15} />{t('settings.syncAll')}</Button><Button disabled={!isDesktop()} title={isDesktop() ? undefined : t('settings.addAccountDesktopOnly')} onClick={() => { setEditing(undefined); setWizardOpen(true); }}><Plus size={15} />{t('settings.addAccount')}</Button></div></div><p className="muted">{t('settings.accountsIntro')}</p><div className="settings-accounts">{accounts.map(account => <div key={account.id}><Avatar text={account.avatar} color={account.color} /><span><strong>{account.name}</strong><small>{account.email}</small>{account.status !== 'demo' && <small className="sync-note">{lastSync(account.id) ? t('settings.lastSync', { time: lastSync(account.id)! }) : t('settings.neverSynced')}{jobsByAccount.get(account.id) ? ` · ${syncJobSummary(jobsByAccount.get(account.id)!)}` : ''}</small>}</span><span className="label">{account.provider === 'microsoft' ? 'Microsoft 365' : account.provider === 'gmail' ? 'Gmail' : account.provider === 'pop3' ? 'POP3' : 'IMAP'} · {statusLabel(account.status)}</span>{account.status !== 'demo' && isDesktop() && <><ImapOptions accountId={account.id} name={account.name} provider={account.provider} /><Button disabled={configuring} aria-label={t('settings.reconfigAria', { name: account.name })} onClick={() => void configure(account.id, account.oauth)}>{account.oauth ? t('settings.reauthorize') : t('settings.connectionSettings')}</Button><Button variant="secondary" className="account-sync" aria-label={t('settings.syncAria', { name: account.name })} disabled={submitting.includes(account.id) || (jobsByAccount.get(account.id) ? isSyncActive(jobsByAccount.get(account.id)!) : false)} onClick={() => void enqueue([account.id])}>{submitting.includes(account.id) ? t('settings.queuing') : jobsByAccount.get(account.id)?.status === 'queued' ? t('settings.queued') : jobsByAccount.get(account.id)?.status === 'running' ? t('settings.syncing') : jobsByAccount.get(account.id)?.status === 'failed' ? t('settings.retry') : t('settings.sync')}</Button>{['queued', 'running'].includes(jobsByAccount.get(account.id)?.status ?? '') && <Button variant="ghost" aria-label={jobsByAccount.get(account.id)?.status === 'running' ? t('settings.stopSyncAria', { name: account.name }) : t('settings.cancelQueuedAria', { name: account.name })} disabled={jobsByAccount.get(account.id)?.cancelRequested} onClick={() => void cancel(jobsByAccount.get(account.id)!.id)}>{jobsByAccount.get(account.id)?.status === 'running' ? t('settings.stopSync') : t('settings.cancelQueued')}</Button>}<Button variant="ghost" className="account-remove" aria-label={pendingDelete === account.id ? t('settings.confirmDeleteAria', { name: account.name }) : t('settings.deleteAccountAria', { name: account.name })} onClick={() => void removeAccount(account.id, account.name)}><Trash2 size={14} />{pendingDelete === account.id ? t('settings.clickAgainToConfirm') : ''}</Button></>}</div>)}</div><div className="sync-queue-note" role="status">{t('settings.queueNote', { running: jobs.filter(job => job.status === 'running').length, queued: jobs.filter(job => job.status === 'queued').length })}</div>{syncError && <div className="inline-error" role="alert">{syncError}<Button variant="ghost" onClick={() => void refreshSync()}>{t('settings.reloadProgress')}</Button></div>}<OutboxPanel /><div className="info-card"><h3>{t('settings.stageHeading')}</h3><p>{t('settings.stageBody')}</p></div>
      {reauthorizing && <OAuthDialog account={reauthorizing.account} initial={reauthorizing.config} onClose={() => setReauthorizing(null)} onSaved={reload} />}
      {wizardOpen && <AccountWizardDialog open initial={editing} tone={nextTone} onClose={() => setWizardOpen(false)} onSaved={reload} />}
      </Tabs.Content>
      <Tabs.Content value="about" className="settings-body"><div className="about-brand"><img src="/logo.png" alt="" /><div><h3>AuuMail</h3><p>{t('about.tagline')}</p></div><span className="label">v{info?.version ?? '0.1.0'}</span></div><div className="info-card"><h3>{t('about.infraHeading')}</h3><p>{t('about.infraBody')}</p><p>{info ? t('about.dbStatus', { state: info.databaseReady ? t('about.dbStateReady') : t('about.dbStateStopped') }) : t('about.browserInfo')}</p></div></Tabs.Content>
    </Tabs.Root>{error && <p className="inline-error" role="alert">{error}</p>}
  </Dialog>;
}
