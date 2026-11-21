import { useEffect, useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { Check } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { InlineConfirmation } from '../../components/ui/InlineConfirmation';
import { useSettingsNavigation } from './settingsNavigation';
import { desktopAi, desktopCredentials } from '../../platform/desktop';
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
  const [loaded, setLoaded] = useState(false);
  const [savedConfig, setSavedConfig] = useState({ baseUrl: '', model: '' });
  const [confirmDelete, setConfirmDelete] = useState(false);
  useSettingsNavigation(baseUrl !== savedConfig.baseUrl || model !== savedConfig.model || key !== '', busy);

  useEffect(() => {
    let active = true;
    void ensureConfig();
    desktopAi.config().then(current => {
      if (active && current) {
        setBaseUrl(current.baseUrl);
        setModel(current.model);
        setSavedConfig({ baseUrl: current.baseUrl, model: current.model });
        setKeySaved(current.keySaved);
      }
    }).catch(currentError => { if (active) setError(errorMessage(currentError)); }).finally(() => { if (active) setLoaded(true); });
    return () => { active = false; };
  }, [ensureConfig]);
  async function save() {
    if (busy) return;
    setBusy(true); setError(''); setSaved(false);
    try {
      await desktopAi.saveConfig(baseUrl, model);
      await useAiStore.getState().ensureConfig(true);
      const refreshed = await desktopAi.config();
      if (refreshed) {
        setBaseUrl(refreshed.baseUrl);
        setModel(refreshed.model);
        setSavedConfig({ baseUrl: refreshed.baseUrl, model: refreshed.model });
        setKeySaved(refreshed.keySaved);
      }
      setSaved(true);
    } catch (saveError) { setError(errorMessage(saveError)); }
    setBusy(false);
  }
  async function saveKey() {
    if (busy || !key.trim()) return;
    setBusy(true); setError(''); setSaved(false);
    try {
      await desktopCredentials.save('ai', 'global', key.trim());
      setKey(''); setKeySaved(true);
    } catch (keyError) { setError(errorMessage(keyError)); }
    setBusy(false);
  }
  async function deleteKey() {
    if (busy) return;
    setConfirmDelete(false);
    setBusy(true); setError(''); setSaved(false);
    try {
      await desktopCredentials.delete('ai', 'global');
      setKeySaved(false);
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
  if (!loaded) return <p role="status">{t('syncOptions.loading')}</p>;
  const disabled = busy || confirmDelete;
  return <div className="ai-settings">
    <div className="setting-heading"><h4>{t('ai.configHeading')}</h4><span>{t('ai.configHint')}</span></div>
    <div className="ai-config-row"><label htmlFor="ai-base-url">{t('ai.baseUrlLabel')}</label><input id="ai-base-url" name="baseUrl" type="url" autoComplete="off" spellCheck={false} value={baseUrl} aria-label={t('ai.baseUrlAria')} placeholder="https://api.openai.com/v1" maxLength={500} disabled={disabled} onChange={event => { setBaseUrl(event.target.value); setSaved(false); }} /></div>
    <div className="ai-config-row"><label htmlFor="ai-model">{t('ai.modelLabel')}</label><input id="ai-model" name="model" autoComplete="off" spellCheck={false} value={model} aria-label={t('ai.modelAria')} placeholder="gpt-4o-mini" maxLength={100} disabled={disabled} onChange={event => { setModel(event.target.value); setSaved(false); }} /></div>
    <div className="ai-config-row"><label htmlFor="ai-key">{t('ai.keyLabel')}</label><input id="ai-key" name="apiKey" type="password" value={key} aria-label={t('ai.keyAria')} placeholder={keySaved ? t('ai.keySavedPlaceholder') : t('ai.keyPlaceholder')} autoComplete="off" disabled={disabled} onChange={event => setKey(event.target.value)} /><Button disabled={disabled || !key.trim()} onClick={() => void saveKey()}>{t('ai.saveKey')}</Button>{keySaved && <Button variant="ghost" disabled={disabled} onClick={() => setConfirmDelete(true)}>{t('ai.deleteKey')}</Button>}</div>
    {confirmDelete && <InlineConfirmation message={t('settings.deleteAiKeyConfirm')} confirmLabel={t('ai.deleteKey')} onConfirm={() => void deleteKey()} onCancel={() => setConfirmDelete(false)} />}
    <div className="ai-config-actions">
      <Button variant="primary" disabled={disabled || !baseUrl.trim() || !model.trim()} onClick={() => void save()}>{t('ai.saveConfig')}</Button>
      <Button variant="secondary" disabled={disabled} onClick={() => void test()}>{t('ai.test')}</Button>
      {busy && <span role="status">{t('settings.operationInProgress')}</span>}
      {saved && <span className="ai-saved" role="status"><Check size={14} />{t('ai.savedNotice')}</span>}
      {keySaved && !saved && <span className="ai-saved" role="status"><Check size={14} />{t('ai.keySaved')}</span>}
      {testResult && <span className="ai-saved" role="status">{testResult}</span>}
    </div>
    {error && <p className="inline-error" role="alert">{error}</p>}
    <p className="muted">{t('ai.configPrivacy')}</p>
  </div>;
}
