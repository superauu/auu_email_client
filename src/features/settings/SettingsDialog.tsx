import { useEffect, useState } from 'react';
import * as Tabs from '@radix-ui/react-tabs';
import { Check, FolderOpen, Monitor, Moon, RefreshCw, Sun } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { Dialog } from '../../components/ui/Dialog';
import { Button } from '../../components/ui/Button';
import { Avatar } from '../../components/ui/Avatar';
import { useThemeStore } from '../../stores/themeStore';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { desktop, isDesktop, type AppInfo } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { resolveSkin, type ThemeMode } from '../../themes/contract';

export function SettingsDialog({ open, onClose, initialTab }: { open: boolean; onClose: () => void; initialTab: 'appearance' | 'accounts' | 'about' }) {
  const { skins, skinId, mode, warnings, loading, setAppearance, rescan } = useThemeStore(useShallow(s => ({ skins: s.skins, skinId: s.skinId, mode: s.mode, warnings: s.warnings, loading: s.loading, setAppearance: s.setAppearance, rescan: s.rescan })));
  const accounts = useWorkspaceStore(s => s.accounts);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState('');
  useEffect(() => { if (open) void desktop.info().then(setInfo).catch(e => setError(errorMessage(e))); }, [open]);
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
      <Tabs.Content value="accounts" className="settings-body"><h3>一个工作区，多个邮箱</h3><p className="muted">以下为演示账号，尚未进行真实授权或收发邮件。</p><div className="settings-accounts">{accounts.map(account => <div key={account.id}><Avatar text={account.avatar} color={account.color} /><span><strong>{account.name}</strong><small>{account.email}</small></span><span className="label">{account.provider === 'microsoft' ? 'Microsoft 365' : account.provider === 'gmail' ? 'Gmail' : 'IMAP'}</span></div>)}</div><div className="info-card"><h3>真实邮箱接入将在下一阶段开放</h3><p>支持方向包括 Microsoft 365、Gmail、IMAP / SMTP 和 POP3。你可以先体验账号切换、统一收件箱与本地草稿。</p></div></Tabs.Content>
      <Tabs.Content value="about" className="settings-body"><div className="about-brand"><img src="/logo.svg" alt="" /><div><h3>AuuMail</h3><p>让邮件井然有序。</p></div><span className="label">v{info?.version ?? '0.1.0'}</span></div><div className="info-card"><h3>基础架构预览</h3><p>当前使用演示数据。星标、归档、已读和事件完成状态仅在本次会话有效；手动保存的演示草稿和外观偏好会保存在本机。</p><p>{info ? `本地数据库已${info.databaseReady ? '准备就绪' : '停止'}，尚未接入真实邮件数据。` : '浏览器预览支持界面和内置皮肤，目录扫描与数据库由桌面版提供。'}</p></div></Tabs.Content>
    </Tabs.Root>{error && <p className="inline-error" role="alert">{error}</p>}
  </Dialog>;
}
