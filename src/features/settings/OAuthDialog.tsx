import { useState } from 'react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import type { OAuthConfig } from '../../domain/oauth';
import type { RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { oauthPhaseText, useOAuthFlow } from './useOAuthFlow';

export function OAuthDialog({ account, initial, lockedProvider, onClose, onSaved }: { account: RealAccountInput; initial?: OAuthConfig; lockedProvider?: OAuthConfig['provider']; onClose: () => void; onSaved: () => Promise<void> }) {
  const [config, setConfig] = useState<OAuthConfig>(initial ?? { provider: lockedProvider ?? 'microsoft', clientId: '', tenant: 'common' });
  const oauth = useOAuthFlow();
  const running = oauth.running;
  const saved = oauth.flow?.state === 'saved';
  // Google 空 Client ID 时由后端使用内置公共应用标识；其余提供商仍需显式填写。
  const builtin = config.provider === 'google' && !config.clientId;
  const finish = async () => { try { oauth.savedRef.current = true; await onSaved(); onClose(); } catch (error) { oauth.setError(`账号已保存，刷新工作区失败：${errorMessage(error)}`); } };
  const close = async () => { if (oauth.flow?.state === 'committing') return; if (saved) { await finish(); return; } if (running) { await oauth.stop(); return; } onClose(); };
  return <Dialog open onOpenChange={value => { if (!value) void close(); }} title={account.id ? '重新授权 IMAP 账号' : '使用 OAuth 添加 IMAP 账号'} description="授权在系统浏览器完成，刷新令牌只保存到系统凭据库。">
    <div className="imap-options">
      <p>{account.email} · {account.name}</p>
      <div className="wizard-fields">
        {!lockedProvider && <label><span>提供商</span><select disabled={!!initial} value={config.provider} onChange={event => setConfig({ ...config, provider: event.target.value === 'google' ? 'google' : 'microsoft' })}><option value="microsoft">Microsoft 365 / Outlook.com</option><option value="google">Gmail / Google Workspace</option></select></label>}
        {!builtin && <label><span>Client ID</span><input autoComplete="off" value={config.clientId} onChange={event => setConfig({ ...config, clientId: event.target.value.trim() })} /></label>}
        {config.provider === 'microsoft' && <label><span>租户</span><input autoComplete="off" value={config.tenant} onChange={event => setConfig({ ...config, tenant: event.target.value.trim() })} /></label>}
      </div>
      {builtin
        ? <p className="muted">使用 AuuMail 内置的公共应用标识（Client ID 已随应用内置），不需要客户端密钥。授权在系统浏览器完成；服务商将展示所需的邮箱访问权限。</p>
        : <p className="muted">使用已注册的公共桌面应用 Client ID，不需要客户端密钥。IMAP 主机将固定为所选提供商的官方地址；本入口仅接入收信与邮件状态同步。服务商会展示所需邮箱访问权限。</p>}
      {!lockedProvider && <p className="muted">Google 使用 Desktop app；Microsoft 使用公共桌面应用并配置 http://localhost/callback 回调及委派的 IMAP 权限。应用注册、测试用户与管理员授权需在服务商控制台完成。</p>}
      {oauth.flow && <p role="status">{oauthPhaseText[oauth.flow.state]}{oauth.flow.folderCount !== null ? `发现 ${oauth.flow.folderCount} 个文件夹。` : ''}</p>}
      {(oauth.error || oauth.flow?.error) && <p className="inline-error" role="alert">{oauth.error || oauth.flow?.error}</p>}
      <div className="compose-footer">
        <Button disabled={oauth.flow?.state === 'committing'} onClick={() => void close()}>{running ? '取消授权' : '关闭'}</Button>
        {saved
          ? <Button variant="primary" onClick={() => void finish()}>刷新账号列表</Button>
          : <Button variant="primary" disabled={running || !builtin && !config.clientId || config.provider === 'microsoft' && !config.tenant} onClick={() => void oauth.begin(account, config)}>在浏览器中授权</Button>}
      </div>
    </div>
  </Dialog>;
}
