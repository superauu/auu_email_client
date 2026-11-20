import { useEffect, useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { Check } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { desktopAi, desktopCredentials, isDesktop } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { useAiStore } from '../../stores/aiStore';
import { useI18n } from '../../i18n/react';

/**
 * AI 服务配置（设置 → AI 助手，桌面专属）：服务地址与模型存 SQLite（v22），
 * API Key 经系统凭据服务保存（provider 'ai'），绝不入库或回显。
 */
export function AiOptions() {
  const t = useI18n();
  const { ensureConfig } = useAiStore(useShallow(s => ({ ensureConfig: s.ensureConfig })));
  const [baseUrl, setBaseUrl] = useState('');
  const [model, setModel] = useState('');
  const [key, setKey] = useState('');
  const [keySaved, setKeySaved] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const [testResult, setTestResult] = useState('');

  useEffect(() => {
    void ensureConfig();
    desktopAi.config().then(current => {
      if (current) { setBaseUrl(current.baseUrl); setModel(current.model); setKeySaved(current.keySaved); }
    }).catch(currentError => setError(errorMessage(currentError)));
  }, [ensureConfig]);

  if (!isDesktop()) return null;
  async function save() {
    if (busy) return;
    setBusy(true); setError(''); setSaved(false);
    try {
      await desktopAi.saveConfig(baseUrl, model);
      await useAiStore.getState().ensureConfig(true);
      const refreshed = await desktopAi.config();
      if (refreshed) { setBaseUrl(refreshed.baseUrl); setModel(refreshed.model); setKeySaved(refreshed.keySaved); }
      setSaved(true);
    } catch (saveError) { setError(errorMessage(saveError)); }
    setBusy(false);
  }
  async function saveKey() {
    if (busy || !key.trim()) return;
    setBusy(true); setError(''); setSaved(false); setKeySaved(false);
    try {
      await desktopCredentials.save('ai', 'global', key.trim());
      setKey(''); setKeySaved(true);
    } catch (keyError) { setError(errorMessage(keyError)); }
    setBusy(false);
  }
  async function deleteKey() {
    if (busy) return;
    setBusy(true); setError(''); setKeySaved(false);
    try {
      await desktopCredentials.delete('ai', 'global');
      await useAiStore.getState().ensureConfig(true);
    } catch (keyError) { setError(errorMessage(keyError)); }
    setBusy(false);
  }
  async function test() {
    if (busy) return;
    setBusy(true); setError(''); setTestResult('');
    try {
      const outcome = await desktopAi.test();
      setTestResult(t('ai.testOk', { count: outcome.text }));
    } catch (testError) { setError(errorMessage(testError)); }
    setBusy(false);
  }
  return <div className="ai-settings">
    <div className="setting-heading"><h3>{t('ai.configHeading')}</h3><span>{t('ai.configHint')}</span></div>
    <div className="ai-config-row"><label htmlFor="ai-base-url">{t('ai.baseUrlLabel')}</label><input id="ai-base-url" value={baseUrl} aria-label={t('ai.baseUrlAria')} placeholder="https://api.openai.com/v1" maxLength={500} disabled={busy} onChange={event => setBaseUrl(event.target.value)} /></div>
    <div className="ai-config-row"><label htmlFor="ai-model">{t('ai.modelLabel')}</label><input id="ai-model" value={model} aria-label={t('ai.modelAria')} placeholder="gpt-4o-mini" maxLength={100} disabled={busy} onChange={event => setModel(event.target.value)} /></div>
    <div className="ai-config-row"><label htmlFor="ai-key">{t('ai.keyLabel')}</label><input id="ai-key" type="password" value={key} aria-label={t('ai.keyAria')} placeholder={keySaved ? t('ai.keySavedPlaceholder') : t('ai.keyPlaceholder')} autoComplete="off" disabled={busy} onChange={event => setKey(event.target.value)} /><Button disabled={busy || !key.trim()} onClick={() => void saveKey()}>{t('ai.saveKey')}</Button>{keySaved && <Button variant="ghost" disabled={busy} onClick={() => void deleteKey()}>{t('ai.deleteKey')}</Button>}</div>
    <div className="ai-config-actions">
      <Button disabled={busy || !baseUrl.trim() || !model.trim()} onClick={() => void save()}>{t('ai.saveConfig')}</Button>
      <Button variant="secondary" disabled={busy} onClick={() => void test()}>{busy ? t('ai.testing') : t('ai.test')}</Button>
      {saved && <span className="ai-saved" role="status"><Check size={14} />{t('ai.savedNotice')}</span>}
      {keySaved && !saved && <span className="ai-saved" role="status"><Check size={14} />{t('ai.keySaved')}</span>}
      {testResult && <span className="ai-saved" role="status">{testResult}</span>}
    </div>
    {error && <p className="inline-error" role="alert">{error}</p>}
    <p className="muted">{t('ai.configPrivacy')}</p>
  </div>;
}
