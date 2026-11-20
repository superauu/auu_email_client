import { create } from 'zustand';
import { desktopAi, isDesktop } from '../platform/desktop';
import { errorMessage } from '../domain/errors';
import type { AiConfig, AiOutcome, AiTaskKind } from '../domain/ai';
import { t } from '../i18n/core';

interface AiState {
  /** 全局配置；null = 未配置（面板引导到设置页）。 */
  config: AiConfig | null;
  configLoaded: boolean;
  running: AiTaskKind | null;
  /** 每个任务保留最近一次结果（面板内切换任务可回看）。 */
  results: Partial<Record<AiTaskKind, AiOutcome>>;
  error: string | null;
  /** force=true 跳过缓存重新拉取（配置保存 / Key 变更后）。 */
  ensureConfig: (force?: boolean) => Promise<void>;
  run: (task: AiTaskKind, accountId: string, messageIds: string[], instruction?: string) => Promise<AiOutcome | null>;
  clearError: () => void;
}

/** AI 桌面专属：非桌面 store 动作诚实拒绝（面板入口同样隐藏）。 */
export const useAiStore = create<AiState>((set, get) => ({
  config: null,
  configLoaded: false,
  running: null,
  results: {},
  error: null,
  async ensureConfig(force = false) {
    if (!isDesktop() || (!force && get().configLoaded)) return;
    set({ configLoaded: true });
    try { set({ config: await desktopAi.config() }); }
    catch (error) { set({ error: errorMessage(error) }); }
  },
  async run(task, accountId, messageIds, instruction) {
    if (!isDesktop()) { set({ error: t('store.aiDesktopOnly') }); return null; }
    set({ running: task, error: null });
    try {
      const outcome = await desktopAi.run(task, accountId, messageIds, instruction);
      set(state => ({ running: null, results: { ...state.results, [task]: outcome } }));
      return outcome;
    } catch (error) {
      set({ running: null, error: errorMessage(error) });
      return null;
    }
  },
  clearError() { set({ error: null }); },
}));
