import { zhCN } from './messages/zh-CN';
import { enUS } from './messages/en-US';

/**
 * i18n 纯内核：字典、语言状态与翻译函数。
 * 不依赖 React、DOM、Tauri 或存储——domain / stores 可安全导入 `t`
 * （校验消息在用户操作时刻以当前语言生成）。语言持久化与 DOM 同步在 `index.ts`。
 */

/** 支持的语言。zh-CN 为源语言（字典 source of truth），en-US 类型强制覆盖全部键。 */
export const languages = ['zh-CN', 'en-US'] as const;
export type Language = (typeof languages)[number];
export type MessageId = keyof typeof zhCN;

const catalogs: Record<Language, Record<MessageId, string>> = { 'zh-CN': zhCN, 'en-US': enUS };
const LANG_LABELS: Record<Language, string> = { 'zh-CN': '中文', 'en-US': 'English' };

/** 语言变化订阅；React 侧经 useSyncExternalStore 桥接。 */
type Listener = () => void;
const listeners = new Set<Listener>();

export function isLanguage(value: unknown): value is Language {
  return typeof value === 'string' && (languages as readonly string[]).includes(value);
}

let current: Language = 'zh-CN';
let version = 0;

export function currentLanguage(): Language {
  return current;
}

/** 界面显示用（本地化）的语言名称。 */
export function languageLabel(language: Language): string {
  return LANG_LABELS[language];
}

/**
 * 翻译当前语言的消息；`{name}` 形式参数按 params 替换。
 * 返回调用时刻语言的文案；语言切换不追溯已生成的错误文本（与既有行为一致）。
 */
export function t(id: MessageId, params?: Record<string, string | number>): string {
  const template = catalogs[current][id] ?? zhCN[id];
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (raw, name: string) =>
    name in params ? String(params[name]) : raw,
  );
}

/** 仅更新内存状态并通知订阅者；持久化由应用侧绑定层负责。 */
export function setLanguage(language: Language): void {
  if (!isLanguage(language) || language === current) return;
  current = language;
  version += 1;
  for (const listener of listeners) listener();
}

/** 显式注入初始语言（应用启动时按持久化偏好 / 系统语言决定）。 */
export function initializeLanguage(language: Language): void {
  if (!isLanguage(language) || language === current) return;
  current = language;
}

export function subscribeI18n(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 供 useSyncExternalStore 使用的快照：语言未变时引用稳定。 */
export function i18nSnapshot(): number {
  return version;
}
