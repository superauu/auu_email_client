import { useEffect, useRef, useState } from 'react';
import { Button } from '../../components/ui/Button';
import { SettingsPageHeading } from './SettingsPageHeading';
import { useSettingsNavigation } from './settingsNavigation';
import type { OAuthConfig } from '../../domain/oauth';
import type { RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { oauthPhaseText, useOAuthFlow } from './useOAuthFlow';
import { useI18n } from '../../i18n/react';

export function OAuthOptions({ account, initial, onComplete, onSaved }: { account: RealAccountInput; initial: OAuthConfig; onComplete: () => void; onSaved: () => Promise<void> }) {
  const t = useI18n();
  const [config, setConfig] = useState(initial);
  const oauth = useOAuthFlow();
  const running = oauth.running;
  const saved = oauth.flow?.state === 'saved';
  // Google / Microsoft 空 Client ID 时由后端使用内置公共应用标识；自定义值优先于内置兜底。
  const builtin = (config.provider === 'google' || config.provider === 'microsoft') && !config.clientId;
  const finish = async () => { try { oauth.savedRef.current = true; await onSaved(); onComplete(); } catch (error) { oauth.setError(t('wizard.savedRefreshFailed', { error: errorMessage(error) })); } };
  // 授权保存成功后立即刷新账号列表并收尾，无需用户再点「刷新账号列表」。
  const autoFinished = useRef(false);
  useEffect(() => {
    if (!saved || autoFinished.current) return;
    autoFinished.current = true;
    void finish();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- finish 依赖本次渲染闭包，仅首次 saved 触发
  }, [saved]);
  const navigate = useSettingsNavigation(!saved && (config.clientId !== initial.clientId || config.tenant !== initial.tenant), running);
  const close = async () => {
    if (oauth.flow?.state === 'committing') return;
    if (saved) { await finish(); return; }
    if (running) { await oauth.stop(); return; }
    navigate?.(onComplete);
  };
  return <>
    <SettingsPageHeading title={t('oauth.titleReauthorize')} description={t('oauth.description')} focus level="h4" />
    <div className="imap-options">
      <div className="wizard-fields">
        <p>{config.provider === 'microsoft' ? 'Microsoft 365 / Outlook.com' : 'Gmail / Google Workspace'}</p>
        {!builtin && <label><span>Client ID</span><input disabled={running || saved} autoComplete="off" value={config.clientId} onChange={event => setConfig({ ...config, clientId: event.target.value.trim() })} /></label>}
        {config.provider === 'microsoft' && <label><span>{t('oauth.tenantLabel')}</span><input disabled={running || saved} autoComplete="off" value={config.tenant} onChange={event => setConfig({ ...config, tenant: event.target.value.trim() })} /></label>}
      </div>
      {builtin
        ? <p className="muted">{t('oauth.builtinNote')}</p>
        : <p className="muted">{t('oauth.customNote')}</p>}
      <p className="muted">{t('oauth.setupNote')}</p>
      {oauth.flow && <p role="status">{oauthPhaseText(oauth.flow.state)}{oauth.flow.folderCount !== null ? t('wizard.oauthFolders', { count: oauth.flow.folderCount }) : ''}</p>}
      {(oauth.error || oauth.flow?.error) && <p className="inline-error" role="alert">{oauth.error || oauth.flow?.error}</p>}
      <div className="settings-form-actions">
        <Button disabled={oauth.flow?.state === 'committing'} onClick={() => void close()}>{running ? t('wizard.cancelAuth') : t('oauth.close')}</Button>
        {saved
          ? <Button variant="primary" onClick={() => void finish()}>{t('wizard.refreshAccounts')}</Button>
          : <Button variant="primary" disabled={running || !builtin && !config.clientId || config.provider === 'microsoft' && !config.tenant} onClick={() => void oauth.begin(account, config)}>{t('wizard.authorizeBrowser')}</Button>}
      </div>
    </div>
  </>;
}
