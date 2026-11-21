import { useEffect, useState } from 'react';
import { DatabaseBackup, HardDriveDownload } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { InlineConfirmation } from '../../components/ui/InlineConfirmation';
import { desktopBackup, isDesktop } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { useI18n } from '../../i18n/react';
import { useSettingsNavigation } from './settingsNavigation';

/**
 * 数据备份与恢复：导出为目录（mail.db 快照 + 原文缓存副本）；
 * 恢复经确认后 staging，重启应用时原子换库生效。系统凭据（密码 / OAuth 刷新令牌 /
 * AI Key）不在备份内，恢复后需重新授权——确认文案明确说明。
 */
export function BackupOptions() {
  const t = useI18n();
  const [pending, setPending] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [confirmRestore, setConfirmRestore] = useState(false);
  useSettingsNavigation(false, busy);

  useEffect(() => {
    if (!isDesktop()) return;
    desktopBackup.pending().then(setPending).catch(() => undefined);
  }, []);

  if (!isDesktop()) return null;
  async function exportBackup() {
    if (busy) return;
    setBusy(true); setError(''); setNotice('');
    try {
      const summary = await desktopBackup.export();
      if (summary) setNotice(t('backup.exportDone', { cacheFiles: summary.cacheFiles }));
    } catch (exportError) { setError(errorMessage(exportError)); }
    setBusy(false);
  }
  async function restore() {
    if (busy) return;
    setConfirmRestore(false);
    setBusy(true); setError(''); setNotice('');
    try {
      const staged = await desktopBackup.stageRestore();
      if (staged) { setPending(true); setNotice(t('backup.restoreStaged')); }
    } catch (restoreError) { setError(errorMessage(restoreError)); }
    setBusy(false);
  }
  return <div className="info-card">
    <h4>{t('backup.heading')}</h4>
    <p>{t('backup.intro')}</p>
    <div className="backup-actions">
      <Button disabled={busy || confirmRestore} onClick={() => void exportBackup()}><DatabaseBackup size={15} aria-hidden />{t('backup.export')}</Button>
      <Button disabled={busy || confirmRestore} onClick={() => setConfirmRestore(true)}><HardDriveDownload size={15} aria-hidden />{t('backup.restore')}</Button>
    </div>
    {confirmRestore && <InlineConfirmation message={t('backup.restoreConfirm')} confirmLabel={t('backup.chooseRestoreDirectory')} onConfirm={() => void restore()} onCancel={() => setConfirmRestore(false)} />}
    {busy && <p role="status">{t('settings.operationInProgress')}</p>}
    {pending && <p className="warning-list" role="status">{t('backup.pendingNotice')}</p>}
    {notice && <p role="status">{notice}</p>}
    {error && <p className="inline-error" role="alert">{error}</p>}
    <p className="muted">{t('backup.privacyNote')}</p>
  </div>;
}
