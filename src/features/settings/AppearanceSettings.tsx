import { useState } from 'react';
import { ALargeSmall, Check, FolderOpen, Languages, Monitor, Moon, RefreshCw, Sun } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { Button } from '../../components/ui/Button';
import { errorMessage } from '../../domain/errors';
import { readerFontLabel, READER_FONT_OPTIONS } from '../../domain/readerFont';
import { useI18n, useLanguage, languageLabel, setLanguage, languages } from '../../i18n/react';
import { desktop, isDesktop, type AppInfo } from '../../platform/desktop';
import { useReaderFontStore } from '../../stores/readerFontStore';
import { useThemeStore } from '../../stores/themeStore';
import { resolveSkin, type ThemeMode } from '../../themes/contract';
import { SettingsPageHeading } from './SettingsPageHeading';

export function AppearanceSettings({ info }: { info: AppInfo | null }) {
  const t = useI18n();
  const language = useLanguage();
  const { skins, skinId, mode, warnings, loading, setAppearance, rescan } = useThemeStore(useShallow(s => ({ skins: s.skins, skinId: s.skinId, mode: s.mode, warnings: s.warnings, loading: s.loading, setAppearance: s.setAppearance, rescan: s.rescan })));
  const { fontId, setFontId } = useReaderFontStore(useShallow(s => ({ fontId: s.fontId, setFontId: s.setFontId })));
  const [error, setError] = useState('');

  return <>
    <SettingsPageHeading title={t('settings.tabAppearance')} />
    <div className="setting-heading"><h4>{t('settings.pickSkin')}</h4><span>{t('settings.skinHint')}</span></div><div className="skin-grid">{skins.map(skin => {
        const sample = resolveSkin([skin], skin.manifest.id, mode, window.matchMedia('(prefers-color-scheme: dark)').matches);
        const selected = skinId === skin.manifest.id;
        return <button key={skin.manifest.id} className={`skin-card ${selected ? 'is-active' : ''}`} aria-pressed={selected} aria-label={t('settings.skinAria', { name: skin.manifest.name })} onClick={() => setAppearance(skin.manifest.id, mode)}>
          <span className="skin-preview" aria-hidden style={{ background: sample.tokens.canvas, borderColor: sample.tokens.border }}><span style={{ background: sample.tokens.rail }} /><span style={{ background: sample.tokens.surface }}><i style={{ background: sample.tokens.accent }} /><i style={{ background: sample.tokens.border }} /><i style={{ background: sample.tokens.border }} /></span><span style={{ background: sample.tokens.surface }}><i style={{ background: sample.tokens.text, opacity: 0.65 }} /><i style={{ background: sample.tokens.border }} /><i style={{ background: sample.tokens.border }} /></span></span>
          <span className="skin-title">{skin.manifest.name}{selected && <Check size={15} aria-hidden />}</span><small>{skin.source === 'builtin' ? t('settings.builtinSkin') : t('settings.skinBy', { author: skin.manifest.author })}</small>
        </button>;
      })}</div>
      <div className="setting-heading"><h4>{t('settings.modeHeading')}</h4><span>{t('settings.modeHint')}</span></div><div className="mode-options">{([{ id: 'light', Icon: Sun, label: t('settings.modeLight') }, { id: 'dark', Icon: Moon, label: t('settings.modeDark') }, { id: 'system', Icon: Monitor, label: t('settings.modeSystem') }] as const).map(({ id, Icon, label }) => <Button key={id} variant={mode === id ? 'primary' : 'secondary'} aria-pressed={mode === id} onClick={() => setAppearance(skinId, id as ThemeMode)}><Icon aria-hidden size={16} />{label}</Button>)}</div>
      <div className="setting-heading"><h4>{t('settings.languageHeading')}</h4><span>{t('settings.languageHint')}</span></div><div className="mode-options">{languages.map(id => <Button key={id} variant={language === id ? 'primary' : 'secondary'} aria-pressed={language === id} onClick={() => setLanguage(id)}><Languages aria-hidden size={16} />{languageLabel(id)}</Button>)}</div>
      <div className="setting-heading"><h4>{t('readerFont.heading')}</h4><span>{t('readerFont.hint')}</span></div><div className="mode-options">{READER_FONT_OPTIONS.map(option => <Button key={option.id} variant={fontId === option.id ? 'primary' : 'secondary'} aria-pressed={fontId === option.id} onClick={() => setFontId(option.id)}><ALargeSmall aria-hidden size={16} />{readerFontLabel(option.id)}</Button>)}</div>
      <div className="skin-directory"><div><h4>{t('settings.customSkinHeading')}</h4><p>{isDesktop() ? t('settings.skinDesktopHint') : t('settings.skinBrowserHint')}</p>{info && <code>{info.skinDirectory}</code>}</div><div><Button disabled={!isDesktop()} onClick={() => { setError(''); void desktop.openSkinDirectory().catch(e => setError(errorMessage(e))); }}><FolderOpen aria-hidden size={15} />{t('settings.openSkinDir')}</Button><Button onClick={() => void rescan()} disabled={loading}><RefreshCw aria-hidden size={15} className={loading ? 'spin' : ''} />{loading ? t('settings.scanning') : t('settings.rescan')}</Button></div></div>
      {error && <p className="inline-error" role="alert">{error}</p>}
      {warnings.length > 0 && <div className="warning-list" role="status">{warnings.map((warning, index) => <p key={index}>{warning}</p>)}</div>}
  </>;
}
