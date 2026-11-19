import { useState } from 'react';
import { BookmarkPlus } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { Dialog } from '../../components/ui/Dialog';
import { captureQuery, savedSearchNameError } from '../../domain/savedSearch';
import { errorMessage } from '../../domain/errors';
import { useSavedSearchStore } from '../../stores/savedSearchStore';
import { useI18n } from '../../i18n/react';
import type { MailQuery } from '../../domain/mail';

/** 保存当前搜索（搜索词 + 范围筛选）为侧栏一键入口；纯本机偏好，桌面与演示通用。 */
export function SaveSearchDialog({ query, onSaved }: { query: MailQuery; onSaved: (name: string) => void }) {
  const t = useI18n();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  if (!query.search.trim()) return null;
  async function save() {
    const invalid = savedSearchNameError(name);
    if (invalid || busy) { if (invalid) setError(invalid); return; }
    setBusy(true); setError('');
    try {
      await useSavedSearchStore.getState().save(name, captureQuery(query));
      const saved = name.trim();
      setOpen(false); setName('');
      onSaved(saved);
    } catch (caught) { setError(errorMessage(caught)); }
    setBusy(false);
  }
  return <><Button variant="ghost" iconOnly aria-label={t('app.saveSearchAria')} title={t('app.saveSearchAria')} onClick={() => { setName(query.search.trim()); setError(''); setOpen(true); }}><BookmarkPlus size={15} /></Button>
    <Dialog open={open} onOpenChange={value => { if (!busy) setOpen(value); }} title={t('searches.dialogTitle')} description={t('searches.dialogDescription')}>
      <div className="folder-manager">
        <div className="folder-create">
          <label className="sr-only" htmlFor="saved-search-name">{t('searches.nameAria')}</label>
          <input id="saved-search-name" value={name} placeholder={t('searches.namePlaceholder')} aria-label={t('searches.nameAria')} maxLength={200} disabled={busy} onChange={event => setName(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') void save(); }} autoFocus />
          <Button disabled={busy || !!savedSearchNameError(name)} onClick={() => void save()}>{t('searches.save')}</Button>
        </div>
        {error && <p className="inline-error" role="alert">{error}</p>}
      </div>
    </Dialog></>;
}
