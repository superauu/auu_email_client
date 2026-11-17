/**
 * i18n 应用绑定层：localStorage 持久化、系统语言检测与 DOM 同步。
 * 纯内核在 `core.ts`（domain 可安全导入）；React 钩子在 `react.ts`。
 */
import { currentLanguage, initializeLanguage, isLanguage, setLanguage as setCoreLanguage, type Language } from './core';

// setLanguage 有意不从 core 再导出：下方包装版追加持久化与 DOM 同步。
export {
  currentLanguage,
  initializeLanguage,
  isLanguage,
  i18nSnapshot,
  languageLabel,
  languages,
  subscribeI18n,
  t,
  type Language,
  type MessageId,
} from './core';

const STORAGE_KEY = 'auumail.ui.lang.v1';

function readStoredLanguage(): Language | undefined {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return undefined;
    const value: unknown = JSON.parse(raw);
    return isLanguage(value) ? value : undefined;
  } catch { return undefined; }
}

/** 首次使用（无持久化偏好）时按系统语言检测；此后以用户显式选择为准。 */
function detectLanguage(): Language {
  const stored = readStoredLanguage();
  if (stored) return stored;
  try {
    return navigator.language?.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US';
  } catch { return 'zh-CN'; }
}

/** 应用启动时确定初始语言并同步 html lang。 */
export function initializeI18n(): void {
  initializeLanguage(detectLanguage());
  document.documentElement.lang = currentLanguage();
}

export function setLanguage(language: Language): void {
  setCoreLanguage(language);
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(language)); } catch { /* 写失败仅影响下次默认 */ }
  document.documentElement.lang = language;
}
