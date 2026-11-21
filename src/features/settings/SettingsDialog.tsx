import { useEffect, useState } from 'react';
import * as Tabs from '@radix-ui/react-tabs';
import { Database, Info, Mail, Palette, Sparkles } from 'lucide-react';
import { Dialog } from '../../components/ui/Dialog';
import { InlineConfirmation } from '../../components/ui/InlineConfirmation';
import { desktop, isDesktop, type AppInfo } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { useI18n } from '../../i18n/react';
import { AccountSettings } from './AccountSettings';
import { AiOptions } from './AiOptions';
import { AppearanceSettings } from './AppearanceSettings';
import { BackupOptions } from './BackupOptions';
import { SettingsPageHeading } from './SettingsPageHeading';
import { SettingsNavigation, type SettingsGuard } from './settingsNavigation';

const settingsTabs = [
  { id: 'accounts', label: 'settings.tabAccounts', Icon: Mail },
  { id: 'appearance', label: 'settings.tabAppearance', Icon: Palette },
  { id: 'ai', label: 'settings.tabAi', Icon: Sparkles },
  { id: 'data', label: 'settings.tabData', Icon: Database },
  { id: 'about', label: 'settings.tabAbout', Icon: Info },
] as const;
export type SettingsTab = (typeof settingsTabs)[number]['id'];

export function SettingsDialog({ initialTab, initialAddAccount = false, onClose }: { initialTab: SettingsTab; initialAddAccount?: boolean; onClose: () => void }) {
  const t = useI18n();
  const [tab, setTab] = useState(initialTab);
  const [startInAdd, setStartInAdd] = useState(initialAddAccount);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState('');
  const [pending, setPending] = useState<{ action: () => void; previousFocus: HTMLElement | null } | null>(null);
  const [guards] = useState(() => new Set<SettingsGuard>());

  useEffect(() => {
    let active = true;
    void desktop.info().then(value => { if (active) setInfo(value); }).catch(error => { if (active) setError(errorMessage(error)); });
    return () => { active = false; };
  }, []);

  function request(action: () => void) {
    const active = Array.from(guards);
    if (pending || active.some(guard => guard.busy)) return;
    if (active.some(guard => guard.dirty)) {
      setPending({ action, previousFocus: document.activeElement instanceof HTMLElement ? document.activeElement : null });
      return;
    }
    action();
  }

  function keepEditing() {
    setPending(null);
    requestAnimationFrame(() => pending?.previousFocus?.focus());
  }

  function selectTab(next: SettingsTab) {
    if (next !== tab) request(() => { setStartInAdd(false); setTab(next); });
  }

  return <Dialog open onOpenChange={value => {
    if (!value) {
      if (pending) keepEditing();
      else request(onClose);
    }
  }} title={t('settings.title')} description={t('settings.description')} className="settings-dialog">
    <SettingsNavigation.Provider value={{ guards, request }}>
      <Tabs.Root value={tab} onValueChange={value => {
        const next = settingsTabs.find(item => item.id === value);
        if (next) selectTab(next.id);
      }} orientation="vertical" activationMode="manual" className="settings-tabs">
        <Tabs.List className="settings-nav" aria-label={t('settings.tabsAria')}>
          {settingsTabs.filter(item => item.id !== 'data' || isDesktop()).map(({ id, label, Icon }) => <Tabs.Trigger key={id} value={id} onKeyDown={event => {
            if (event.key === 'Enter' || event.key === ' ') {
              // The confirmation moves focus to Cancel. Prevent the same keystroke from clicking it.
              event.preventDefault();
              if (!event.repeat) selectTab(id);
            }
          }}><Icon size={17} aria-hidden />{t(label)}</Tabs.Trigger>)}
        </Tabs.List>
        <div className="settings-main">
          {pending && <div className="settings-navigation-confirm"><InlineConfirmation message={t('settings.unsavedChanges')} confirmLabel={t('settings.discardChanges')} cancelLabel={t('settings.keepEditing')} onCancel={keepEditing} onConfirm={() => { setPending(null); pending.action(); }} /></div>}
          <Tabs.Content value="accounts" className="settings-body" inert={!!pending}><AccountSettings initialAdd={startInAdd} /></Tabs.Content>
          <Tabs.Content value="appearance" className="settings-body" inert={!!pending}><AppearanceSettings info={info} /></Tabs.Content>
          <Tabs.Content value="ai" className="settings-body" inert={!!pending}>
            <SettingsPageHeading title={t('settings.tabAi')} />
            {isDesktop() ? <AiOptions /> : <p className="muted">{t('settings.aiDesktopHint')}</p>}
          </Tabs.Content>
          <Tabs.Content value="data" className="settings-body" inert={!!pending}>
            <SettingsPageHeading title={t('settings.tabData')} />
            <BackupOptions />
          </Tabs.Content>
          <Tabs.Content value="about" className="settings-body" inert={!!pending}>
            <SettingsPageHeading title={t('settings.tabAbout')} />
            <div className="about-brand"><img src="/logo.png" alt="" width="48" height="48" /><div><h4>AuuMail</h4><p>{t('about.tagline')}</p></div><span className="label">v{info?.version ?? '0.1.0'}</span></div>
            <div className="info-card"><h4>{t('about.infraHeading')}</h4><p>{info ? t('about.dbStatus', { state: info.databaseReady ? t('about.dbStateReady') : t('about.dbStateStopped') }) : t('about.browserInfo')}</p></div>
            {error && <p className="inline-error" role="alert">{error}</p>}
          </Tabs.Content>
        </div>
      </Tabs.Root>
    </SettingsNavigation.Provider>
  </Dialog>;
}
