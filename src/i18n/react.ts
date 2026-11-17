import { useSyncExternalStore } from 'react';
import {
  currentLanguage,
  i18nSnapshot,
  languageLabel,
  languages,
  setLanguage,
  subscribeI18n,
  t,
  type Language,
  type MessageId,
} from './index';

export type { Language, MessageId };
export { currentLanguage, languageLabel, languages, setLanguage, t };

/** 当前语言快照（变化时触发订阅组件重渲）。 */
export function useLanguage(): Language {
  useSyncExternalStore(subscribeI18n, i18nSnapshot, i18nSnapshot);
  return currentLanguage();
}

/** 翻译函数；组件内调用即可在语言切换时重渲（t 引用恒定）。 */
export function useI18n(): typeof t {
  useLanguage();
  return t;
}
