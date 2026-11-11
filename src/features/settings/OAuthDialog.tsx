import { useEffect, useRef, useState } from 'react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import { useOAuthStore } from '../../stores/oauthStore';
import type { OAuthConfig, OAuthFlow } from '../../domain/oauth';
import type { RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
const phases: Record<OAuthFlow['state'], string> = { waiting: '等待系统浏览器授权…', exchanging: '正在取得授权…', verifying: '正在验证 IMAP 登录和文件夹…', committing: '正在安全保存账号…', saved: '授权和 IMAP 验证通过，账号已保存。', failed: '授权未完成。', cancelled: '授权已取消，账号未保存。' };
export function OAuthDialog({ account, initial, onClose, onSaved }: { account: RealAccountInput; initial?: OAuthConfig; onClose: () => void; onSaved: () => Promise<void> }) {
  const [config, setConfig] = useState<OAuthConfig>(initial ?? { provider: 'microsoft', clientId: '', tenant: 'common' });
  const [flow, setFlow] = useState<OAuthFlow | null>(null);
  const [flowId, setFlowId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const saved = useRef(false);
  const running = busy || !!flowId && !flow || !!flow && !['saved', 'failed', 'cancelled'].includes(flow.state);
  async function finish() { try { await onSaved(); saved.current = true; onClose(); } catch (error) { setError(`账号已保存，刷新工作区失败：${errorMessage(error)}`); } }
  useEffect(() => {
    if (!flowId) return;
    let active = true; let timer: ReturnType<typeof setTimeout> | undefined;
    async function poll() {
      try {
        const value = await useOAuthStore.getState().read(flowId!);
        if (!active) return;
        setFlow(value);
        if (value.state === 'saved') return;
        if (['failed', 'cancelled'].includes(value.state)) return;
      } catch (error) { if (active) setError(errorMessage(error)); }
      if (active) timer = setTimeout(() => void poll(), 1000);
    }
    void poll();
    return () => { active = false; clearTimeout(timer); if (!saved.current) void useOAuthStore.getState().cancel(flowId).catch(() => {}); };
  }, [flowId]);
  async function start() { setBusy(true); setError(''); setFlow(null); try { setFlowId(await useOAuthStore.getState().start(account, config)); } catch (error) { setError(errorMessage(error)); } setBusy(false); }
  async function close() { if (busy || flow?.state === 'committing') return; if (flow?.state === 'saved') { await finish(); return; } if (running && flowId) { try { setFlow(await useOAuthStore.getState().cancel(flowId)); } catch (error) { setError(errorMessage(error)); return; } } onClose(); }
  return <Dialog open onOpenChange={value => { if (!value) void close(); }} title={account.id ? '重新授权 IMAP 账号' : '使用 OAuth 添加 IMAP 账号'} description="授权在系统浏览器完成，刷新令牌只保存到系统凭据库。">
    <div className="imap-options">
      <p>{account.email} · {account.name}</p>
      <div className="wizard-fields">
        <label><span>提供商</span><select disabled={running || !!initial} value={config.provider} onChange={event => setConfig({ ...config, provider: event.target.value === 'google' ? 'google' : 'microsoft' })}><option value="microsoft">Microsoft 365 / Outlook.com</option><option value="google">Gmail / Google Workspace</option></select></label>
        <label><span>Client ID</span><input autoComplete="off" disabled={running} value={config.clientId} onChange={event => setConfig({ ...config, clientId: event.target.value.trim() })} /></label>
        {config.provider === 'microsoft' && <label><span>租户</span><input autoComplete="off" disabled={running} value={config.tenant} onChange={event => setConfig({ ...config, tenant: event.target.value.trim() })} /></label>}
      </div>
      <p className="muted">使用已注册的公共桌面应用 Client ID，不需要客户端密钥。IMAP 主机将固定为所选提供商的官方地址；本入口仅接入收信与邮件状态同步。服务商会展示所需邮箱访问权限。</p>
      <p className="muted">Google 使用 Desktop app；Microsoft 使用公共桌面应用并配置 http://localhost/callback 回调及委派的 IMAP 权限。应用注册、测试用户与管理员授权需在服务商控制台完成。</p>
      {flow && <p role="status">{phases[flow.state]}{flow.folderCount !== null ? `发现 ${flow.folderCount} 个文件夹。` : ''}</p>}
      {(error || flow?.error) && <p className="inline-error" role="alert">{error || flow?.error}</p>}
      <div className="compose-footer"><Button disabled={flow?.state === 'committing'} onClick={() => void close()}>{running ? '取消授权' : '关闭'}</Button>{flow?.state === 'saved' ? <Button variant="primary" onClick={() => void finish()}>刷新账号列表</Button> : <Button variant="primary" disabled={running || !config.clientId || !config.tenant} onClick={() => void start()}>在浏览器中授权</Button>}</div>
    </div>
  </Dialog>;
}
