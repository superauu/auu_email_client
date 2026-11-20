import { useState } from 'react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import type { OAuthConfig } from '../../domain/oauth';
import type { RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { oauthPhaseText, useOAuthFlow } from './useOAuthFlow';
import { useI18n } from '../../i18n/react';

export function OAuthDialog({ account, initial, lockedProvider, onClose, onSaved }: { account: RealAccountInput; initial?: OAuthConfig; lockedProvider?: OAuthConfig['provider']; onClose: () => void; onSaved: () => Promise<void> }) {
  const t = useI18n();
  const [config, setConfig] = useState<OAuthConfig>(initial ?? { provider: lockedProvider ?? 'microsoft', clientId: '', tenant: 'common' });
  const oauth = useOAuthFlow();
  const running = oauth.running;
  const saved = oauth.flow?.state === 'saved';
  // Google / Microsoft 空 Client ID 时由后端使用内置公共应用标识；自定义值优先于内置兜底。
  const builtin = (config.provider === 'google' || config.provider === 'microsoft') && !config.clientId;
  const finish = async () => { try { oauth.savedRef.current = true; await onSaved(); onClose(); } catch (error) { oauth.setError(t('wizard.savedRefreshFailed', { error: errorMessage(error) })); } };
  const close = async () => { if (oauth.flow?.state === 'committing') return; if (saved) { await finish(); return; } if (running) { await oauth.stop(); return; } onClose(); };
  return <Dialog open onOpenChange={value => { if (!value) void close(); }} title={account.id ? t('oauth.titleReauthorize') : t('oauth.titleAdd')} description={t('oauth.description')}>
    <div className="imap-options">
      <p>{account.email} · {account.name}</p>
      <div className="wizard-fields">
        {!lockedProvider && <label><span>{t('oauth.providerLabel')}</span><select disabled={!!initial} value={config.provider} onChange={event => setConfig({ ...config, provider: event.target.value === 'google' ? 'google' : 'microsoft' })}><option value="microsoft">Microsoft 365 / Outlook.com</option><option value="google">Gmail / Google Workspace</option></select></label>}
        {!builtin && <label><span>Client ID</span><input autoComplete="off" value={config.clientId} onChange={event => setConfig({ ...config, clientId: event.target.value.trim() })} /></label>}
        {config.provider === 'microsoft' && <label><span>{t('oauth.tenantLabel')}</span><input autoComplete="off" value={config.tenant} onChange={event => setConfig({ ...config, tenant: event.target.value.trim() })} /></label>}
      </div>
      {builtin
        ? <p className="muted">{t('oauth.builtinNote')}</p>
        : <p className="muted">{t('oauth.customNote')}</p>}
      {!lockedProvider && <p className="muted">{t('oauth.setupNote')}</p>}
      {oauth.flow && <p role="status">{oauthPhaseText(oauth.flow.state)}{oauth.flow.folderCount !== null ? t('wizard.oauthFolders', { count: oauth.flow.folderCount }) : ''}</p>}
      {(oauth.error || oauth.flow?.error) && <p className="inline-error" role="alert">{oauth.error || oauth.flow?.error}</p>}
      <div className="compose-footer">
        <Button disabled={oauth.flow?.state === 'committing'} onClick={() => void close()}>{running ? t('wizard.cancelAuth') : t('oauth.close')}</Button>
        {saved
          ? <Button variant="primary" onClick={() => void finish()}>{t('wizard.refreshAccounts')}</Button>
          : <Button variant="primary" disabled={running || !builtin && !config.clientId || config.provider === 'microsoft' && !config.tenant} onClick={() => void oauth.begin(account, config)}>{t('wizard.authorizeBrowser')}</Button>}
      </div>
    </div>
  </Dialog>;
}
