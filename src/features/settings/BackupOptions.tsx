import { useEffect, useState } from 'react';
import { DatabaseBackup, HardDriveDownload } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { desktopBackup, isDesktop } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { useI18n } from '../../i18n/react';

/**
 * 数据备份与恢复（设置 → 关于）：导出为目录（mail.db 快照 + 原文缓存副本）；
 * 恢复经确认后 staging，重启应用时原子换库生效。系统凭据（密码 / OAuth 刷新令牌 /
 * AI Key）不在备份内，恢复后需重新授权——确认文案明确说明。
 */
export function BackupOptions() {
  const t = useI18n();
  const [pending, setPending] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');

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
    if (!window.confirm(t('backup.restoreConfirm'))) return;
    setBusy(true); setError(''); setNotice('');
    try {
      const staged = await desktopBackup.stageRestore();
      if (staged) { setPending(true); setNotice(t('backup.restoreStaged')); }
    } catch (restoreError) { setError(errorMessage(restoreError)); }
    setBusy(false);
  }
  return <div className="info-card">
    <h3>{t('backup.heading')}</h3>
    <p>{t('backup.intro')}</p>
    <div className="backup-actions">
      <Button disabled={busy} onClick={() => void exportBackup()}><DatabaseBackup size={15} />{t('backup.export')}</Button>
      <Button variant="secondary" disabled={busy} onClick={() => void restore()}><HardDriveDownload size={15} />{t('backup.restore')}</Button>
    </div>
    {pending && <p className="warning-list" role="status">{t('backup.pendingNotice')}</p>}
    {notice && <p role="status">{notice}</p>}
    {error && <p className="inline-error" role="alert">{error}</p>}
    <p className="muted">{t('backup.privacyNote')}</p>
  </div>;
}
