import { useEffect, useState } from 'react';
import * as Tabs from '@radix-ui/react-tabs';
import { Check, FolderOpen, Monitor, Moon, Plus, RefreshCw, Sun, Trash2 } from 'lucide-react';
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
import { isSyncActive, syncJobSummary } from '../../domain/sync';

export function SettingsDialog({ open, onClose, initialTab }: { open: boolean; onClose: () => void; initialTab: 'appearance' | 'accounts' | 'about' }) {
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
    catch (removeError) { setError(`删除账号 ${name} 失败：${errorMessage(removeError)}`); }
  }
  const eligible = accounts.filter(account => account.status !== 'demo' && account.provider === 'imap');
  const jobsByAccount = new Map(jobs.map(job => [job.accountId, job]));
  const realCount = accounts.filter(account => account.status !== 'demo').length;
  const nextTone = realCount % 3 === 0 ? 'teal' : realCount % 3 === 1 ? 'blue' : 'amber' as const;
  const lastSync = (id: string) => syncStates?.[id] ? new Intl.DateTimeFormat('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(new Date(syncStates[id])) : null;
  return <Dialog open={open} onOpenChange={value => { if (!value) onClose(); }} title="让工作区，更像你" description="外观、账号与应用信息。" className="settings-dialog">
    <Tabs.Root defaultValue={initialTab} className="settings-tabs"><Tabs.List className="tab-list" aria-label="设置分类"><Tabs.Trigger value="appearance">外观与皮肤</Tabs.Trigger><Tabs.Trigger value="accounts">邮箱账号</Tabs.Trigger><Tabs.Trigger value="about">关于</Tabs.Trigger></Tabs.List>
      <Tabs.Content value="appearance" className="settings-body"><div className="setting-heading"><h3>选择一款皮肤</h3><span>切换后即时生效</span></div><div className="skin-grid">{skins.map(skin => {
        const sample = resolveSkin([skin], skin.manifest.id, mode, window.matchMedia('(prefers-color-scheme: dark)').matches);
        const selected = skinId === skin.manifest.id;
        return <button key={skin.manifest.id} className={`skin-card ${selected ? 'is-active' : ''}`} aria-pressed={selected} aria-label={`使用皮肤 ${skin.manifest.name}`} onClick={() => setAppearance(skin.manifest.id, mode)}>
          <span className="skin-preview" style={{ background: sample.tokens.canvas, borderColor: sample.tokens.border }}><span style={{ background: sample.tokens.rail }} /><span style={{ background: sample.tokens.surface }}><i style={{ background: sample.tokens.accent }} /><i style={{ background: sample.tokens.border }} /><i style={{ background: sample.tokens.border }} /></span><span style={{ background: sample.tokens.surface }}><i style={{ background: sample.tokens.text, opacity: 0.65 }} /><i style={{ background: sample.tokens.border }} /><i style={{ background: sample.tokens.border }} /></span></span>
          <span className="skin-title">{skin.manifest.name}{selected && <Check size={15} />}</span><small>{skin.source === 'builtin' ? '内置皮肤' : `来自 ${skin.manifest.author}`}</small>
        </button>;
      })}</div>
      <div className="setting-heading"><h3>明暗模式</h3><span>皮肤与系统外观独立设置</span></div><div className="mode-options">{([{ id: 'light', Icon: Sun, label: '浅色' }, { id: 'dark', Icon: Moon, label: '深色' }, { id: 'system', Icon: Monitor, label: '跟随系统' }] as const).map(({ id, Icon, label }) => <Button key={id} variant={mode === id ? 'primary' : 'secondary'} aria-pressed={mode === id} onClick={() => setAppearance(skinId, id as ThemeMode)}><Icon size={16} />{label}</Button>)}</div>
      <div className="skin-directory"><div><h3>添加你喜欢的皮肤</h3><p>{isDesktop() ? '将皮肤文件夹放入目录，再重新扫描即可使用。' : '桌面版支持从本机目录安装独立皮肤。'}</p>{info && <code>{info.skinDirectory}</code>}</div><div><Button disabled={!isDesktop()} onClick={() => { setError(''); void desktop.openSkinDirectory().catch(e => setError(errorMessage(e))); }}><FolderOpen size={15} />打开皮肤目录</Button><Button onClick={() => void rescan()} disabled={loading}><RefreshCw size={15} className={loading ? 'spin' : ''} />{loading ? '正在扫描…' : '重新扫描'}</Button></div></div>
      {warnings.length > 0 && <div className="warning-list" role="status">{warnings.map((warning, index) => <p key={index}>{warning}</p>)}</div>}
      </Tabs.Content>
      <Tabs.Content value="accounts" className="settings-body"><div className="setting-heading"><h3>一个工作区，多个邮箱</h3><div className="account-actions"><Button disabled={!isDesktop() || !eligible.some(account => !submitting.includes(account.id) && !(jobsByAccount.get(account.id) && isSyncActive(jobsByAccount.get(account.id)!)))} onClick={() => void enqueue(eligible.map(account => account.id))}><RefreshCw size={15} />同步全部</Button><Button disabled={!isDesktop()} title={isDesktop() ? undefined : '添加真实账号需要在桌面版中进行'} onClick={() => { setEditing(undefined); setWizardOpen(true); }}><Plus size={15} />添加 IMAP 账号</Button></div></div><p className="muted">演示账号仅在本机模拟；真实账号连接验证通过后才会保存，密码只写入系统凭据库。</p><div className="settings-accounts">{accounts.map(account => <div key={account.id}><Avatar text={account.avatar} color={account.color} /><span><strong>{account.name}</strong><small>{account.email}</small>{account.status !== 'demo' && <small className="sync-note">{lastSync(account.id) ? `上次同步 ${lastSync(account.id)}` : '尚未同步'}{jobsByAccount.get(account.id) ? ` · ${syncJobSummary(jobsByAccount.get(account.id)!)}` : ''}</small>}</span><span className="label">{account.provider === 'microsoft' ? 'Microsoft 365' : account.provider === 'gmail' ? 'Gmail' : 'IMAP'} · {account.status === 'demo' ? '演示' : account.status === 'connected' ? '已连接' : account.status === 'offline' ? '离线' : '需要重新授权'}</span>{account.status !== 'demo' && isDesktop() && <><ImapOptions accountId={account.id} name={account.name} /><Button disabled={configuring} aria-label={`重新配置 ${account.name}`} onClick={() => void configure(account.id, account.oauth)}>{account.oauth ? '重新授权' : '连接设置'}</Button><Button variant="secondary" className="account-sync" aria-label={`同步 ${account.name}`} disabled={account.provider !== 'imap' || submitting.includes(account.id) || (jobsByAccount.get(account.id) ? isSyncActive(jobsByAccount.get(account.id)!) : false)} onClick={() => void enqueue([account.id])}>{submitting.includes(account.id) ? '正在排队…' : jobsByAccount.get(account.id)?.status === 'queued' ? '排队中' : jobsByAccount.get(account.id)?.status === 'running' ? '同步中…' : jobsByAccount.get(account.id)?.status === 'failed' ? '重试' : '同步'}</Button>{['queued', 'running'].includes(jobsByAccount.get(account.id)?.status ?? '') && <Button variant="ghost" aria-label={`${jobsByAccount.get(account.id)?.status === 'running' ? '停止同步' : '取消排队'} ${account.name}`} disabled={jobsByAccount.get(account.id)?.cancelRequested} onClick={() => void cancel(jobsByAccount.get(account.id)!.id)}>{jobsByAccount.get(account.id)?.status === 'running' ? '停止同步' : '取消排队'}</Button>}<Button variant="ghost" className="account-remove" aria-label={pendingDelete === account.id ? `确认删除 ${account.name}` : `删除账号 ${account.name}`} onClick={() => void removeAccount(account.id, account.name)}><Trash2 size={14} />{pendingDelete === account.id ? '再点一次确认' : ''}</Button></>}</div>)}</div><div className="sync-queue-note" role="status">{jobs.filter(job => job.status === 'running').length} 个账号同步中 · {jobs.filter(job => job.status === 'queued').length} 个排队中。关闭设置后任务继续运行。</div>{syncError && <div className="inline-error" role="alert">{syncError}<Button variant="ghost" onClick={() => void refreshSync()}>重新读取进度</Button></div>}<div className="info-card"><h3>当前阶段边界</h3><p>首次同步最近邮件，之后增量更新；同步设置可开启全部文件夹、历史下载和自动同步。最多两个账号并行，任务可停止，失败批次保留。附件可从完整原文另存为；发送与服务器草稿同步尚未接入。演示账号不会连接服务器。</p></div>
      {reauthorizing && <OAuthDialog account={reauthorizing.account} initial={reauthorizing.config} onClose={() => setReauthorizing(null)} onSaved={reload} />}
      {wizardOpen && <AccountWizardDialog open initial={editing} tone={nextTone} onClose={() => setWizardOpen(false)} onSaved={reload} />}
      </Tabs.Content>
      <Tabs.Content value="about" className="settings-body"><div className="about-brand"><img src="/logo.svg" alt="" /><div><h3>AuuMail</h3><p>让邮件井然有序。</p></div><span className="label">v{info?.version ?? '0.1.0'}</span></div><div className="info-card"><h3>基础架构预览</h3><p>工作区可以包含演示数据和已同步的真实邮件。星标、归档、已读和草稿保存在本机；事件完成状态仍仅本次会话有效；桌面版使用本地数据库，浏览器仅提供演示。</p><p>{info ? `本地数据库已${info.databaseReady ? '准备就绪' : '停止'}，支持读取已同步邮件和本地草稿。` : '浏览器预览支持界面和内置皮肤，目录扫描与数据库由桌面版提供。'}</p></div></Tabs.Content>
    </Tabs.Root>{error && <p className="inline-error" role="alert">{error}</p>}
  </Dialog>;
}
